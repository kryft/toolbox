> **WARNING — GENERATED PROJECT AID.**
>
> This file is a convenience summary extracted from the vendored MCP 2025-11-25
> specification.  The vendored specification and `schema.ts` remain authoritative.
> When in doubt, consult the source files in this directory.

# Minimal Stdio MCP Server — Specification Summary

Target: a server that runs over **stdio**, declares the **tools** capability, and
exposes a single man-page lookup tool.

---

## 1. Base JSON-RPC Message Shapes

Source: `overview.md` § *Messages*; `schema.ts` (JSON-RPC category)

All messages **MUST** follow [JSON-RPC 2.0](https://www.jsonrpc.org/specification).

### Request

```json
{ "jsonrpc": "2.0", "id": <string | number>, "method": <string>, "params": <object>? }
```

- `jsonrpc` **MUST** be `"2.0"` (`schema.ts` `JSONRPC_VERSION`).
- `id` **MUST** be a string or number; it **MUST NOT** be `null`
  (`overview.md` § *Requests*).
- The ID **MUST NOT** have been previously used by the same sender in the session
  (`overview.md` § *Requests*).
- `params` is optional but most methods require it.

Schema reference: `JSONRPCRequest` in `schema.ts`.

### Result Response

```json
{ "jsonrpc": "2.0", "id": <same as request>, "result": <object> }
```

- `id` **MUST** match the request's `id` (`overview.md` § *Result Responses*).
- `result` **MUST** be present and may be any JSON object
  (`overview.md` § *Result Responses*).

Schema reference: `JSONRPCResultResponse` in `schema.ts`.

### Error Response

```json
{ "jsonrpc": "2.0", "id": <same as request | omitted>, "error": { "code": <int>, "message": <string>, "data": <any>? } }
```

- `id` **MUST** match the request's `id`, except when the request was so malformed
  the ID could not be read (`overview.md` § *Error Responses*).
- `error.code` **MUST** be an integer (`overview.md` § *Error Responses*).
- `error.message` SHOULD be a concise single sentence (`schema.ts` `Error`).

Standard error codes (`schema.ts`):

| Constant              | Value  |
|-----------------------|--------|
| `PARSE_ERROR`         | -32700 |
| `INVALID_REQUEST`     | -32600 |
| `METHOD_NOT_FOUND`    | -32601 |
| `INVALID_PARAMS`      | -32602 |
| `INTERNAL_ERROR`      | -32603 |

Schema reference: `JSONRPCErrorResponse` in `schema.ts`.

### Notification

```json
{ "jsonrpc": "2.0", "method": <string>, "params": <object>? }
```

- **MUST NOT** include an `id` (`overview.md` § *Notifications*).
- The receiver **MUST NOT** send a response (`overview.md` § *Notifications*).

Schema reference: `JSONRPCNotification` in `schema.ts`.

---

## 2. Stdio Transport Framing

Source: `transports.md` § *stdio*

- The client launches the server as a subprocess.
- Server reads JSON-RPC messages from **stdin** and writes them to **stdout**.
- Messages are delimited by **newlines** (`\n`).
- Messages **MUST NOT** contain embedded newlines (`transports.md` § *stdio*).
- Messages **MUST** be UTF-8 encoded (`transports.md` first paragraph).
- The server **MUST NOT** write anything to `stdout` that is not a valid MCP message
  (`transports.md` § *stdio*).
- The server **MAY** write UTF-8 strings to **stderr** for logging
  (`transports.md` § *stdio*).
- The client **SHOULD NOT** assume `stderr` output indicates error conditions
  (`transports.md` § *stdio*).

---

## 3. Initialization Lifecycle and Version Negotiation

Source: `lifecycle.md` § *Initialization* and § *Version Negotiation*

The initialization phase **MUST** be the first interaction between client and server
(`lifecycle.md` § *Initialization*).

### Step 1 — Client sends `initialize` request

The client sends a JSON-RPC request with `method: "initialize"`:

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "initialize",
  "params": {
    "protocolVersion": "<string>",
    "capabilities": <ClientCapabilities>,
    "clientInfo": { "name": "<string>", "version": "<string>" }
  }
}
```

- `protocolVersion` — the latest version the client supports
  (`lifecycle.md` § *Version Negotiation*).
- `clientInfo` — implements `Implementation` from `schema.ts` (`name: string`, `version: string`,
  optional `title`, `description`, `icons`, `websiteUrl`).

Schema reference: `InitializeRequest`, `InitializeRequestParams`, `Implementation`
in `schema.ts`.

### Step 2 — Server responds with `InitializeResult`

The server responds with its own version and capabilities:

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "protocolVersion": "<string>",
    "capabilities": <ServerCapabilities>,
    "serverInfo": { "name": "<string>", "version": "<string>" }
  }
}
```

- `protocolVersion` — if the server supports the client's version, it **MUST**
  respond with the **same** version. Otherwise it **MUST** respond with a different
  version it supports (`lifecycle.md` § *Version Negotiation*). This **SHOULD** be the
  server's latest version.
- `serverInfo` — implements `Implementation` from `schema.ts` (required fields:
  `name: string`, `version: string`).
- `instructions` — optional string that MAY be added to the system prompt
  (`schema.ts` `InitializeResult`).

Schema reference: `InitializeResult` in `schema.ts`.

The tagged `schema.ts` defines the internal constant
`LATEST_PROTOCOL_VERSION = "DRAFT-2025-v3"`, while the published protocol
version and specification prose use `"2025-11-25"`.

This project targets and advertises `"2025-11-25"`. Treat the draft-named
constant in the vendored schema as an upstream inconsistency, not as the wire
version to implement.

### Step 3 — Client sends `notifications/initialized`

After receiving a successful `initialize` response, the client **MUST** send an
`initialized` notification (`lifecycle.md` § *Initialization*). See § 4 below.

### Ordering constraints

- The client **SHOULD NOT** send requests other than `ping` before the server has
  responded to `initialize` (`lifecycle.md` § *Initialization*).
- The server **SHOULD NOT** send requests other than `ping` and logging before
  receiving `notifications/initialized` (`lifecycle.md` § *Initialization*).

---

## 4. `notifications/initialized`

Source: `lifecycle.md` § *Initialization*; `schema.ts` `InitializedNotification`

```json
{ "jsonrpc": "2.0", "method": "notifications/initialized" }
```

- This is a **notification** — no `id`, no response expected.
- `params` is optional (`schema.ts` `InitializedNotification.params?`).
- The client **MUST** send this after successful initialization
  (`lifecycle.md` § *Initialization*).
- The spec restricts only what the server *sends* before this point:
  "The server **SHOULD NOT** send requests other than pings and logging before
  receiving the `initialized` notification" (`lifecycle.md` § *Initialization*).
  It does not normatively require the server to wait before *responding to* client
  requests.
- **Project policy**: this server should treat `notifications/initialized` as the
  gate before processing any tool requests.

---

## 5. Required Server Metadata and Capabilities

Source: `lifecycle.md` § *Capability Negotiation*; `tools.md` § *Capabilities*;
`schema.ts` `ServerCapabilities`, `Implementation`

### Server Info (`serverInfo`)

Required fields in the `InitializeResult` (`schema.ts` `Implementation`):

| Field     | Type    | Required |
|-----------|---------|----------|
| `name`    | string  | MUST     |
| `version` | string  | MUST     |

Optional: `title`, `description`, `icons`, `websiteUrl`.

### Server Capabilities

A server that exposes tools **MUST** declare the `tools` capability
(`tools.md` § *Capabilities`):

```json
{
  "capabilities": {
    "tools": {}
  }
}
```

- `tools.listChanged` — boolean, optional. Present if the server will emit
  `notifications/tools/list_changed` (`schema.ts` `ServerCapabilities.tools`).
  For a static tool set, this can be omitted.

The `ServerCapabilities` interface (`schema.ts`) also defines optional capabilities:
`experimental`, `logging`, `completions`, `prompts`, `resources`, `tasks`.
None are required for a minimal tools-only server.

---

## 6. `tools/list`

Source: `tools.md` § *Listing Tools*; `schema.ts` `ListToolsRequest`,
`ListToolsResult`, `Tool`, `PaginatedRequest`, `PaginatedResult`

### Request

```json
{
  "jsonrpc": "2.0",
  "id": 2,
  "method": "tools/list",
  "params": { "cursor": "<opaque string>?" }
}
```

- `params` and `cursor` are optional (`PaginatedRequestParams` in `schema.ts`).
- If no `cursor` is provided, the server returns the first page.

### Response

```json
{
  "jsonrpc": "2.0",
  "id": 2,
  "result": {
    "tools": [ <Tool>[] ],
    "nextCursor": "<opaque string>?"
  }
}
```

- `tools` — array of `Tool` objects (`ListToolsResult` in `schema.ts`).
- `nextCursor` — present if more results follow (`PaginatedResult` in `schema.ts`).

### Tool object

Required fields (`schema.ts` `Tool`, which extends `BaseMetadata`):

| Field           | Type     | Required |
|-----------------|----------|----------|
| `name`          | string   | MUST     |
| `inputSchema`   | object   | MUST     |

Optional fields: `title`, `description`, `outputSchema`, `annotations`, `execution`,
`icons`, `_meta`.

`inputSchema` structure (`schema.ts` `Tool.inputSchema`):

```ts
{
  $schema?: string;
  type: "object";
  properties?: { [key: string]: object };
  required?: string[];
}
```

- `type` **MUST** be `"object"` (`tools.md` § *Tool*).
- For tools with no parameters: `{ "type": "object" }` or
  `{ "type": "object", "additionalProperties": false }`
  (`tools.md` § *Tool*).

### Tool naming rules (`tools.md` § *Tool Names*)

- **SHOULD** be 1–128 characters.
- **SHOULD** be case-sensitive.
- **SHOULD** use only `[A-Za-z0-9_.-]`.
- **SHOULD NOT** contain spaces, commas, or other special characters.
- **SHOULD** be unique within a server.

---

## 7. `tools/call`

Source: `tools.md` § *Calling Tools*; `schema.ts` `CallToolRequest`,
`CallToolRequestParams`, `CallToolResult`

### Request

```json
{
  "jsonrpc": "2.0",
  "id": 3,
  "method": "tools/call",
  "params": {
    "name": "<tool name>",
    "arguments": { "<param>": <value>, ... }?
  }
}
```

- `name` — the tool name as declared in `tools/list` (required,
  `CallToolRequestParams` in `schema.ts`).
- `arguments` — optional key-value map; values are `unknown`
  (`CallToolRequestParams` in `schema.ts`).
- `params` may also include `_meta` and `task` fields (via
  `TaskAugmentedRequestParams` in `schema.ts`), but these are not needed for a
  minimal server.

### Success Response

```json
{
  "jsonrpc": "2.0",
  "id": 3,
  "result": {
    "content": [ <ContentBlock>[] ],
    "isError": false?
  }
}
```

- `content` — array of content blocks (`CallToolResult` in `schema.ts`).
  For text results, use `TextContent` (see below).
- `isError` — optional boolean. Defaults to `false` when omitted
  (`schema.ts` `CallToolResult.isError`).
- `structuredContent` — optional JSON object for structured results
  (`schema.ts` `CallToolResult.structuredContent`).

### `TextContent` (the content block we need)

Source: `schema.ts` `TextContent`

```json
{ "type": "text", "text": "<string>" }
```

Optional: `annotations`, `_meta`.

### Tool execution errors vs protocol errors

Source: `tools.md` § *Error Handling*

There are **two** mechanisms:

1. **Protocol-level error response** (`JSONRPCErrorResponse`): Use for issues
   with *finding* the tool, unsupported operations, or malformed requests.
   Example: unknown tool name → return `error.code: -32602`
   (`INVALID_PARAMS` in `schema.ts`).

2. **Tool execution error** (result with `isError: true`): Use for errors that
   originate *from* the tool itself (e.g., API failure, input validation error).
   The LLM can see `isError: true` and potentially self-correct.

From `schema.ts` `CallToolResult.isError` doc comment:

> Any errors that originate from the tool SHOULD be reported inside the result
> object, with `isError` set to true, *not* as an MCP protocol-level error response.
> However, any errors in *finding* the tool, an error indicating that the server
> does not support tool calls, or any other exceptional conditions, should be
> reported as an MCP error response.

Clients **SHOULD** provide tool execution errors to language models to enable
self-correction (`tools.md` § *Error Handling`).

---

## 8. Required Ordering and State Transitions

Source: `lifecycle.md` § *Lifecycle Phases*

The protocol defines three phases:

```
[Initialization] → [Operation] → [Shutdown]
```

### State machine (server perspective)

1. **Awaiting initialize**
   - Server starts, reads from stdin.
   - First message **MUST** be an `initialize` request
     (`lifecycle.md` § *Initialization*).
   - Server responds with `InitializeResult` or an error.

2. **Awaiting initialized notification**
   - Server has sent the `initialize` response.
   - The spec restricts only what the server *sends* during this window:
     "The server **SHOULD NOT** send requests other than pings and logging before
     receiving the `initialized` notification" (`lifecycle.md` § *Initialization*).
     It does not normatively restrict the server from *responding to* client requests
     before `initialized`.
   - **Project policy**: this server treats `notifications/initialized` as the gate
     before processing tool requests.
   - Client **MUST** send `notifications/initialized`
     (`lifecycle.md` § *Initialization*).

3. **Operation**
   - Server handles `tools/list`, `tools/call`, `ping`, etc.
   - Both parties **MUST** respect the negotiated protocol version and only use
     negotiated capabilities (`lifecycle.md` § *Operation`).

4. **Shutdown**
   - For stdio, the client **SHOULD** close stdin, then wait for the server to
     exit, then `SIGTERM`, then `SIGKILL` if needed
     (`lifecycle.md` § *Shutdown* § *stdio`).
   - The server **MAY** initiate shutdown by closing its output stream and exiting
     (`lifecycle.md` § *Shutdown* § *stdio`).

### Version mismatch handling

- If the client sends a version the server does not support, the server **MUST**
  respond with a version it supports (`lifecycle.md` § *Version Negotiation`).
- If the client does not support the version in the server's response, it
  **SHOULD** disconnect (`lifecycle.md` § *Version Negotiation`).

### Timeouts

- Implementations **SHOULD** establish timeouts for all sent requests
  (`lifecycle.md` § *Timeouts`).

---

## 9. Server Security Requirements

Source: `tools.md` § *Security Considerations*

The specification lists four server **MUST** requirements:

| Requirement            | Status for this project                        |
|------------------------|------------------------------------------------|
| Validate all tool inputs | **Immediately relevant** — validate the decoded
  arguments before invoking the tool. This may be implemented directly for the
  man-page tool; a general-purpose JSON Schema validator is not initially required. |
| Implement proper access controls | **Minimal local model** — the server is
  available only to the process that launches it over stdio and runs with that
  user's permissions. No additional authentication is currently planned. |
| Rate limit tool invocations | **Not initially implemented** — accepted as a
  known compliance gap for the local single-client learning prototype. Revisit
  if calls can consume substantial resources or the server gains remote or
  multi-client access. |
| Sanitize tool outputs | **Immediately relevant** — ensure returned process
  output is suitable for exposure to the client and does not accidentally
  disclose unintended data. JSON escaping is handled by the serializer;
  the existing output limit is a separate size-control measure. |

The remaining items in the section are client-side **SHOULD** requirements
(user confirmation, input display, result validation, timeouts, audit logging)
and are not server responsibilities.

---

## 10. Quick-Reference: Minimal Message Flow

```
Client                          Server
  |                               |
  |── initialize ────────────────>|  (JSON-RPC request)
  |<── InitializeResult ──────────|  (result response)
  |── notifications/initialized →|  (notification, no id)
  |                               |
  |── tools/list ────────────────>|  (after initialized)
  |<── ListToolsResult ───────────|
  |                               |
  |── tools/call ────────────────>|
  |<── CallToolResult ────────────|  (or error response)
  |                               |
  |── (close stdin) ─────────────>|  (shutdown)
  |                               |  (exit)
```

All messages are newline-delimited JSON on stdin/stdout. Logging may go to stderr.
