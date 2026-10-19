// The connection to shader-language-server (https://github.com/antaalt/shader-sense), running in the browser: its
// WASI build in workers (web/lsp/), talking LSP with CodeMirror's client.
//
// The server must read a visual's code as the app compiles it, with the code shared by all visuals around it (see
// `render::language_server_context`). The connection does that between the client and the server:
// - documents are sent wrapped with that code, and whole at each change (the server is told to expect it);
// - positions in requests are moved down by the lines put before the visual's code, and moved back up in answers;
// - what answers point at in the shared code is dropped: errors, definitions… are only shown in the visual's code;
// - definitions are sent back as locations, the only kind the client reads (the server sends links).

// Answers or parts of them pointing outside the visual's code.
const DROP = Symbol("drop");

// The server's settings. Its own diagnostics stop at warnings: hints would be noise next to the compile errors.
const SERVER_CONFIG = { validate: true, symbols: true, severity: "warning" };

/// Starts the server, and returns the client's transport to it.
///
/// `context`: per language ("wgsl", "glsl"), the code to put `before` and `after` a visual's. `onStatus(state,
/// detail)`: "loading", "ready", "error" or "exited", with details to show.
export function startServer({ workerUrl, wasmUrl, context, onStatus }) {
    const worker = new Worker(workerUrl, { type: "module" });
    const handlers = new Set();
    // Per open document: lines added before the visual's code, and its number of lines.
    const documents = new Map();
    // Requests waiting for an answer: their document.
    const requests = new Map();

    const languageOf = (uri) => (uri.endsWith(".frag") ? "glsl" : "wgsl");
    const wrap = (uri, text) => {
        const { before, after } = context[languageOf(uri)];
        documents.set(uri, { offset: lineCount(before) - 1, lines: lineCount(text) });
        return before + text + after;
    };

    const toServer = (message) => {
        const params = message.params;
        const uri = params?.textDocument?.uri;
        const document = uri && documents.get(uri);
        switch (message.method) {
            case "textDocument/didOpen":
                params.textDocument.text = wrap(uri, params.textDocument.text);
                return message;
            case "textDocument/didChange":
                params.contentChanges = params.contentChanges.map((change) =>
                    change.range ? moveDown(change, document) : { text: wrap(uri, change.text) },
                );
                return message;
            case "textDocument/didClose":
                documents.delete(uri);
                return message;
        }
        if (uri && !document && message.id !== undefined) {
            // About a document closed meanwhile (another visual was opened): nothing to answer.
            return { reply: { jsonrpc: "2.0", id: message.id, result: null } };
        }
        if (document) {
            if (message.id !== undefined) {
                requests.set(message.id, uri);
            }
            message.params = moveDown(params, document);
        }
        return message;
    };

    const fromServer = (message) => {
        // The answer to `initialize`: ask for whole documents at each change, so they can be wrapped.
        const capabilities = message.result?.capabilities;
        if (capabilities) {
            const sync = capabilities.textDocumentSync;
            capabilities.textDocumentSync = typeof sync === "object" ? { ...sync, change: 1 } : 1;
        }
        if (message.method === undefined && requests.has(message.id)) {
            const document = documents.get(requests.get(message.id));
            requests.delete(message.id);
            if (!document) {
                // The document was closed before the server answered (it handles messages concurrently): the
                // answer, or its error ("File not watched"), is about nothing open anymore.
                return { jsonrpc: "2.0", id: message.id, result: null };
            }
            if (message.result) {
                const result = moveUp(message.result, document);
                message.result = result === DROP ? null : asLocations(withoutPlaceholders(result));
            }
        } else if (message.method === "textDocument/publishDiagnostics") {
            const document = documents.get(message.params.uri);
            if (!document) {
                return null;
            }
            message.params.diagnostics = moveUp(message.params.diagnostics, document);
        }
        return message;
    };

    const deliver = (message) => {
        const text = JSON.stringify(message);
        for (const handler of handlers) {
            handler(text);
        }
    };
    worker.addEventListener("message", (event) => {
        const message = event.data;
        if (!message?.jsonrpc) {
            if (message?.type === "status") {
                onStatus(message.state, message.detail ?? "");
            }
            return;
        }
        const forwarded = fromServer(message);
        if (forwarded) {
            deliver(forwarded);
        }
    });
    worker.addEventListener("error", (event) => onStatus("error", `Server worker failed: ${event.message}`));
    worker.postMessage({
        type: "start",
        wasmUrl,
        args: ["shader-language-server", "--config", JSON.stringify(SERVER_CONFIG), "--glsl", "--wgsl"],
        env: { RUST_LOG: "shader_language_server=warn,shader_sense=warn" },
    });

    return {
        send: (text) => {
            const message = toServer(JSON.parse(text));
            if (message.reply) {
                // Answered here: later, like an answer from the server.
                setTimeout(() => deliver(message.reply));
            } else {
                worker.postMessage(message);
            }
        },
        subscribe: (handler) => handlers.add(handler),
        unsubscribe: (handler) => handlers.delete(handler),
    };
}

// A name the server puts in the code it analyzes, which can come back as a suggestion.
const PLACEHOLDER = "__SHADER_SENSE__";

/// Removes the server's placeholder from suggestions.
function withoutPlaceholders(result) {
    const keep = (item) => item?.label !== PLACEHOLDER;
    if (Array.isArray(result)) {
        return result.filter(keep);
    }
    if (Array.isArray(result.items)) {
        return { ...result, items: result.items.filter(keep) };
    }
    return result;
}

/// Links to definitions (`targetUri`…) as plain locations (`uri`, `range`).
function asLocations(result) {
    const location = (item) =>
        item?.targetUri ? { uri: item.targetUri, range: item.targetSelectionRange ?? item.targetRange } : item;
    return Array.isArray(result) ? result.map(location) : location(result);
}

function lineCount(text) {
    return text.split("\n").length;
}

function isPosition(value) {
    return typeof value.line === "number" && typeof value.character === "number" && Object.keys(value).length === 2;
}

/// Positions in the visual's code to positions in the wrapped document.
function moveDown(value, document) {
    if (Array.isArray(value)) {
        return value.map((item) => moveDown(item, document));
    }
    if (value && typeof value === "object") {
        if (isPosition(value)) {
            return { line: value.line + document.offset, character: value.character };
        }
        return Object.fromEntries(Object.entries(value).map(([key, item]) => [key, moveDown(item, document)]));
    }
    return value;
}

/// Positions in the wrapped document back to positions in the visual's code. Anything pointing outside of it is
/// dropped: a whole answer, or an item of a list (one error, one definition…).
function moveUp(value, document) {
    if (Array.isArray(value)) {
        return value.map((item) => moveUp(item, document)).filter((item) => item !== DROP);
    }
    if (value && typeof value === "object") {
        if (isPosition(value)) {
            const line = value.line - document.offset;
            return line >= 0 && line < document.lines ? { line, character: value.character } : DROP;
        }
        const result = {};
        for (const [key, item] of Object.entries(value)) {
            const moved = moveUp(item, document);
            if (moved === DROP) {
                return DROP;
            }
            result[key] = moved;
        }
        return result;
    }
    if (typeof value === "string") {
        // Hovers tell where a name is declared.
        return value.replace(/Defined in this file, line (\d+)/g, (_, number) => {
            const line = Number(number) - 1 - document.offset;
            return line >= 0 && line < document.lines
                ? `Defined on line ${line + 1}`
                : "Built into the app (see Available inputs below the editor)";
        });
    }
    return value;
}

// The tags and attributes kept in the server's docs, rendered from Markdown. They can quote the visual's code, which
// may come from a shared link: nothing else can run or load anything.
const SAFE_TAGS = new Set(["P", "PRE", "CODE", "EM", "STRONG", "B", "I", "UL", "OL", "LI", "BR", "HR", "SPAN", "DIV", "H1", "H2", "H3", "H4", "BLOCKQUOTE"]);

/// Keeps only plain formatting in HTML.
export function sanitizeHTML(html) {
    const template = document.createElement("template");
    template.innerHTML = html;
    const clean = (node) => {
        for (const child of [...node.children]) {
            if (!SAFE_TAGS.has(child.tagName)) {
                child.replaceWith(document.createTextNode(child.textContent));
                continue;
            }
            for (const attribute of [...child.attributes]) {
                if (attribute.name !== "class") {
                    child.removeAttribute(attribute.name);
                }
            }
            clean(child);
        }
    };
    clean(template.content);
    return template.innerHTML;
}
