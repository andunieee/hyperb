# The hyperb protocol

hyperb renderers display [hypereffect](https://crates.io/crates/hypereffect)
apps. This document specifies how a renderer is pointed at an app (the
`hyperb://` address) and how it talks to it over each of the three kinds of
session. The hypereffect message protocol itself is not covered here.

Status: all of it is implemented in `hyperb-core` (`address.rs`, `http.rs`,
and `net/iroh.rs`, `net/wasm.rs` behind the `iroh` and `wasm` features). The
desktop, terminal and Android renderers have every kind of session, and the web build has HTTP and remote wasm (`net/wasm_js.rs`, see
section 4). The demo server serves its apps over HTTP and iroh, some of them
also as wasm modules (`/<app>.wasm`), and prints their addresses.

## 1. Addresses

```
hyperb://<authority>/<path>?<query>
hyperb:///<file path>.wasm#<route>
```

A single scheme, `hyperb`, covers every kind of session. The renderer picks
the session from the address, in this order:

| #   | Address                                                      | Session     |
| --- | ---------                                                    | ---------   |
| 1   | The part after `hyperb://` starts with `/` (empty authority) | local wasm  |
| 2   | The host is 64 hexadecimal characters                        | iroh        |
| 3   | A path ending in `.wasm`                                     | remote wasm |
| 4   | Anything else                                                | HTTP        |

Rule 3 applies to the HTTP addresses of section 1.2 (and to `hyperb+http`
and `hyperb+https`), so rules 3 and 4 are decided by the path alone.

Rule 2 is unambiguous because a DNS label has at most 63 characters, so a
64-character host is never a domain name. The hex is case-insensitive;
renderers lowercase it.

### 1.1 Scheme variants

- `hyperb+https://` and `hyperb+http://` force the HTTP scheme. They exist for
  edge cases only (for example a TLS-terminating proxy on `localhost`, or a
  plain-http server on a public name); nothing should generate them by
  default. They are errors with an iroh or local address.
- `web+hyperb://` is an alias of `hyperb://`, because web browsers only let
  pages handle schemes starting with `web+`. Renderers accept and handle
  both (section 1.7), and display `hyperb://`.
- `http://` and `https://` are accepted as aliases of the matching variants,
  so existing URLs keep working.
- Input without a scheme is read as `hyperb://`. A bare absolute path
  (`/opt/apps/a.wasm`) or `~/`-path is read as a local wasm address.
- Scheme names are case-insensitive. Any other scheme is rejected.

### 1.2 HTTP addresses

`hyperb://host[:port]/path[?query]` is an HTTP-with-SSE session (section 2)
at that URL. The transport scheme is:

- `http` for `localhost` (and `*.localhost`), for IP addresses (IPv4 and
  IPv6, any range) and for `*.onion` hosts;
- `https` for everything else.

### 1.3 Remote wasm addresses

An HTTP address whose path ends in `.wasm` (case-insensitive) is not a server:
the renderer downloads the file (at most 64 MiB) and runs it locally as in
section 4. The fragment is the route, as for local files. The transport
scheme is chosen as in 1.2. Remote wasm sessions are never shown as secure
(section 1.6).

Renderers may keep the module of an app the user marked as a favorite and
run it from that copy, without the network; they download it again in the
background on each session, so the next one gets any update. The copy is
keyed by the address without the fragment, so every route shares it.

### 1.4 Iroh addresses

`hyperb://<endpoint-id>[/path][?query]`, where `<endpoint-id>` is the
server's iroh endpoint id (its ed25519 public key) in 64 hex characters.
There is no port. The path and query select the app (section 3).

### 1.5 Local wasm addresses

`hyperb:///<absolute path to a .wasm file>[#<route>]`. The authority is empty,
as in `file:///`. The path is percent-decoded to get the file; it must not end
in `/`. The optional fragment is the route inside the app, like the path of a
remote address (it is not sent anywhere, so it cannot be confused with the
file path).

For safety, renderers should open local wasm addresses only when the user
enters them (address bar, file picker, command line, their own history and
favorites), never from a link or redirect coming from an app or a remote
page (section 1.7).

### 1.6 Canonical form and security

Renderers normalize an address into a URL whose scheme names the transport,
and use that to open the session and to show the tab:

| Input                         | Canonical                       |
| -------                       | -----------                     |
| `hyperb://example.com/app`    | `https://example.com/app`       |
| `hyperb://localhost:9001/app` | `http://localhost:9001/app`     |
| `hyperb://<64 hex>/app`       | `iroh://<64 hex lowercase>/app` |
| `hyperb:///opt/a.wasm#/menu`  | `wasm:///opt/a.wasm#/menu`      |

Renderers show a session as secure when nobody on the network can read or
alter it and no code from a remote party runs on this machine:

| Session                          | Secure                                                      |
| ---------                        | --------                                                    |
| `https` app                      | yes                                                         |
| `iroh`                           | yes                                                         |
| local wasm                       | yes (no network, and the user chose the file)               |
| `http` app                       | no (`.onion` is encrypted by Tor but still shown as `http`) |
| remote wasm, over `https` or not | no (foreign code)                                           |

### 1.7 Links

Renderers register as the system's handler of `hyperb://` and
`web+hyperb://` links where the platform allows it:

- Desktop: the `x-scheme-handler/hyperb` and `x-scheme-handler/web+hyperb`
  MIME types (`crates/desktop/com.hyperb.app.desktop`, installed by
  `just desktop-install`). Links open in a new tab of the running window.
- Android: a `VIEW` intent filter for both schemes.
- Web: `navigator.registerProtocolHandler("web+hyperb", …)`, after the user
  asks for it on the welcome page; the browser sends links to the page as
  `?link=<address>`. Pages cannot handle `hyperb` itself.

The terminal renderer is not a handler: it runs the one address it is given.

An address that comes from a link must have a scheme, and must not be a
local wasm address (`address::from_link`).

## 2. HTTP session

Implemented. See the module documentation of `crates/core/src/http.rs`:
the renderer POSTs the `hello` line with its public key in `hyperb-key`; the
server answers with a session id in `hyperb-session`; the renderer opens an
`text/event-stream` GET signed with `hyperb-signature` (its signature of the
session id); every later renderer line is an authenticated POST. Each server
line `tag json` is one SSE event named `tag` with the JSON as data.

## 3. Iroh session

1. The renderer connects to the endpoint id with the ALPN
   `hyperb/0<path><?query>`: `hyperb/0` followed by the address's path
   (nothing for the root path) and its query, e.g. `hyperb/0/restaurant`. An
   ALPN is at most 255 bytes, so longer addresses are rejected. Servers list
   the ALPNs of the apps they serve when binding their endpoint; an unknown
   ALPN fails the handshake. Iroh's discovery and relays find the server.
   The connection is encrypted and the server is authenticated by its
   endpoint id.
2. The renderer's identity (`identity.rs`) is the key of its iroh endpoint.
   The server takes it from the connection, as the value of the `hyperb-key`
   header in HTTP sessions, and the handshake proves it, so there are no
   headers and no signatures.
3. The renderer opens one bidirectional stream. Both directions carry
   hypereffect lines (`tag json\n`, UTF-8), the same lines as in the HTTP
   session. The first line is the renderer's `hello`; QUIC only announces the
   stream to the server when data arrives on it, so servers accept it after
   the `hello` is sent.
4. The session ends when either side closes the stream or the connection.

## 4. Wasm session

For local and remote wasm addresses. The renderer loads a core wasm module
(binary, or text format) on wasmtime and runs it in a sandbox. The module and
the renderer call one function each to pass lines to the other:

|                                          |                                                                                     |
| ---                                      | ---                                                                                 |
| export `memory`                          | the module's linear memory                                                          |
| export `alloc(len: i32) -> i32`          | returns a buffer of `len` bytes in `memory`; the module owns it after `receive`     |
| export `receive(ptr: i32, len: i32)`     | the renderer sends one or more lines (UTF-8, each ending with a line feed) at `ptr` |
| import `hyperb.send(ptr: i32, len: i32)` | the module sends one or more lines, at `ptr` in `memory`                            |

The lines are the hypereffect lines of the other sessions, the `hello`
first. `receive` runs to completion and may call `send` any number of times.
Lines reach the module as the renderer wrote them. The module is stateless to
the renderer: there is no identity, signature or session, and a new instance
is created for each connection. It has no other imports (no WASI): no clock,
files or network.

Limits: 256 MiB of memory, and about 5 billion instructions of fuel for each
`receive`. Going over them, or any trap, closes the session with the error.
The module cannot tick; it only reacts to lines.

The web renderer runs modules on the browser's WebAssembly engine instead
of wasmtime, with the same imports and exports, except that it accepts only
the binary format, cannot open local files, and has no fuel or memory
limits: the module runs on the page's thread, so one that never returns
freezes the page.

## 5. Sessions in the renderer

Whatever the kind, a session is the `Connection` of `crates/core/src/net`:
after `Event::Open` the renderer sends `hello`, lines from the app arrive as
`Event::Data`, and `Event::Closed(reason)` ends it. A renderer that cannot
run a kind of session (for example the browser build cannot read local files)
closes it with a descriptive reason.
