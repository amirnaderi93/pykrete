# LSP multiplexer — design

## Goal

dathon-lsp becomes the **single LSP** the editor talks to. Internally it
embeds a Python language server (basedpyright today; Astral's `ty` once
it reaches stable 1.0) as a child process, forwards general-Python
requests to it, and merges the child's responses with dathon's own
schema-aware results.

This replaces the iteration-39 **co-activation** model (two LSPs the
user installs and the editor merges) with **multiplexing** (one LSP;
dathon-lsp does the merging internally).

Why: co-activation needs two installs, has no clean PyCharm story, and
can't deliver zero-import ergonomics because the Python LSP reads
`.dpy` files straight from disk. Multiplexing fixes all three — the
editor sees one server, dathon-lsp controls exactly what the embedded
engine sees, and PyCharm gets full support through its LSP client.

## Topology

```
        ┌─────────────┐   LSP/JSON-RPC over stdio   ┌──────────────┐
 editor │  VS Code /  │ ─────────────────────────── │  dathon-lsp  │
        │  PyCharm    │                             │ (the proxy)  │
        └─────────────┘                             └──────┬───────┘
                                                            │ LSP/JSON-RPC
                                                            │ over a pipe
                                                     ┌──────┴───────┐
                                                     │  basedpyright │
                                                     │  (child proc) │
                                                     └──────────────┘
```

The editor only ever knows about dathon-lsp. The child is an
implementation detail.

## Virtual documents

`.dpy` files are valid Python today, so the child can analyze them
almost as-is. The one transform: dathon's magic names (`Schema`,
`DataFrame`, `col`) aren't defined anywhere the child can see. So for
every `.dpy` document, dathon-lsp constructs a **virtual document**:

```
<injected preamble: from dathon import *  (+ the stub body inline)>
<original .dpy content, verbatim>
```

The child receives the virtual document under the same URI. The
preamble is a fixed number of leading lines, so **position mapping is a
constant line offset** — `virtual_line = real_line + PREAMBLE_LINES`
and back. No complex source map needed (that changes only if dathon
ever gains non-Python syntax — the Volar model handles that case, and
the offset mapping generalizes to a full source map then).

The user's real file has **zero imports**; the child still sees a
fully-resolvable Python module.

## Message routing

dathon-lsp's main loop now selects over two inbound streams: the
editor and the child. Routing rules:

| Message | Handling |
|---|---|
| `initialize` (editor→us) | Answer with merged capabilities (ours ∪ child's). Spawn + initialize the child first so we know its capabilities. |
| `textDocument/didOpen/didChange/didClose` | Apply to our doc store; forward the **virtual** document to the child. |
| `textDocument/publishDiagnostics` (child→us) | Map positions back, tag source, merge with dathon's diagnostics for that URI, publish the union. |
| `textDocument/hover`, `completion`, `definition`, `references`, `rename`, `signatureHelp`, formatting, … (editor→us) | Fan out: run dathon's handler **and** forward to the child. Merge the two responses (concatenate completion lists, stack hover sections, prefer dathon for schema positions). |
| `textDocument/documentSymbol`, `codeAction` | dathon answers; optionally also fold in the child's. |
| Any request the child needs to send us (`workspace/configuration`, `client/registerCapability`, …) | Proxy through to the editor, proxy the reply back. |
| `shutdown` / `exit` | Forward to the child, wait for it to exit, then exit ourselves. |

Requests carry IDs. dathon-lsp keeps an **id-remap table** so a child
response can be matched to the editor request that triggered it (and
vice versa for child→editor requests). IDs are rewritten on the way
through to avoid collisions between the two id spaces.

## Concurrency

`lsp-server` gives us a crossbeam channel for the editor side. The
child side is a spawned process with stdin/stdout pipes. A dedicated
**reader thread** frames the child's stdout (`Content-Length` headers),
deserializes each message, and pushes it onto a channel. The main loop
`select!`s over `editor_rx` and `child_rx`. Writes to the child go
through a `Mutex<ChildStdin>`.

This keeps the model close to today's single-threaded handler loop —
one extra thread for the child reader, everything else sequential.

## basedpyright discovery

Phased:

1. **Now** — locate the engine via, in order: a `dathon.pythonServer.path`
   setting, then `basedpyright-langserver` / `pyright-langserver` on
   `PATH`. If none is found, dathon-lsp runs in **dathon-only mode** —
   exactly today's behavior, no child, no Python features. Never a hard
   error; degrade gracefully.
2. **Later** — the VS Code extension bundles a basedpyright build so
   there's nothing for the user to install.
3. **Eventually** — swap basedpyright for `ty` once it ships stable
   1.0: a Rust language server, no Node runtime, making dathon-lsp +
   engine a single self-contained native stack.

## Fallback / resilience

- No engine found → dathon-only mode (current behavior).
- Child crashes mid-session → log, publish a warning, fall back to
  dathon-only for the rest of the session; optionally restart with
  backoff.
- Child is slow → dathon's own results are never blocked on the child;
  we publish dathon diagnostics immediately and amend with the child's
  when they arrive.

## Build order

This is multi-iteration. The slices, smallest-shippable first:

1. **Foundation** *(done)* — spawn the child, frame JSON-RPC both ways,
   forward lifecycle (`initialize`/`initialized`/`shutdown`/`exit`) and
   text-sync notifications with the virtual-document transform, merge
   `publishDiagnostics`. Engine located via setting / PATH; dathon-only
   fallback.
2. **Request fan-out** *(done)* — hover, completion, definition: query
   both, merge.
3. **Capability negotiation** *(done)* — manual `initialize` handshake;
   advertise dathon's capabilities ∪ the child's, for an allowlist of
   methods dathon-lsp can proxy correctly (`signatureHelp`,
   `references`). Relay the child's notifications to the editor.
4. **child→editor request proxying** *(done)* — `workspace/configuration`,
   `client/registerCapability`, `window/workDoneProgress/create` etc.
   are proxied through to the real editor and the reply routed back (a
   second id-remap table), so the engine picks up the editor's Python
   settings instead of a stub. Known gap: notifications the editor
   sends as a result of a dynamic registration aren't forwarded to the
   child yet.
5. **Wider passthrough** — `rename`, `documentHighlight`, semantic
   tokens, …: each needs its own virtual↔editor coordinate transform
   before it joins the capability allowlist.
6. **Bundle the engine** — *VS Code done:* the extension ships
   basedpyright in `node_modules` and passes dathon-lsp a
   `node <langserver.js> --stdio` launch spec via
   `initializationOptions.pythonServer`. Needs Node.js on `PATH`;
   degrades to dathon-only otherwise. *Pending:* PyCharm setup docs.
7. **`ty` swap** when it's stable — a native Rust engine drops the
   Node.js dependency entirely.

## Out of scope (for now)

- Incremental text sync — still FULL sync.
- Non-Python dathon syntax — the constant-offset position mapping
  upgrades to a real source map if/when that happens.
- Embedding the engine in-process — it stays a subprocess; the LSP
  boundary is the whole point (swap engines without touching dathon).
