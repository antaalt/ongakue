// Coordinator of the shader-language-server "process".
//
// It speaks LSP with the editor through postMessage (one JSON-RPC object per message), and translates it to the
// Content-Length framed stdio stream the server expects. It never blocks, as it must keep spawning workers for the
// wasm threads & feeding stdin while these threads are waiting on Atomics.
import { CONTROL_SIZE, StdinWriter, createStdinBuffer, readImportedMemoryLimits } from "./wasi.js";

const stdinBuffer = createStdinBuffer();
const controlBuffer = new SharedArrayBuffer(CONTROL_SIZE * Int32Array.BYTES_PER_ELEMENT);
const stdin = new StdinWriter(stdinBuffer);
const encoder = new TextEncoder();
const decoder = new TextDecoder();
const threads = new Map();
let serverProcess = null;
let initializeRequestId = null;
let stdoutBuffer = new Uint8Array(0);

function status(state, detail) {
    self.postMessage({ type: "status", state, detail });
}

function log(stream, text) {
    self.postMessage({ type: "log", stream, text });
}

function writeMessage(message) {
    if (message.method === "initialize") {
        initializeRequestId = message.id;
    }
    const content = encoder.encode(JSON.stringify(message));
    stdin.write(encoder.encode(`Content-Length: ${content.length}\r\n\r\n`));
    stdin.write(content);
}

function findHeaderEnd(buffer) {
    for (let i = 0; i + 3 < buffer.length; i++) {
        if (buffer[i] === 13 && buffer[i + 1] === 10 && buffer[i + 2] === 13 && buffer[i + 3] === 10) {
            return i;
        }
    }
    return -1;
}

function readStdout(data) {
    const merged = new Uint8Array(stdoutBuffer.length + data.length);
    merged.set(stdoutBuffer);
    merged.set(data, stdoutBuffer.length);
    stdoutBuffer = merged;
    for (;;) {
        const headerEnd = findHeaderEnd(stdoutBuffer);
        if (headerEnd < 0) {
            return;
        }
        const header = decoder.decode(stdoutBuffer.subarray(0, headerEnd));
        const match = /Content-Length:\s*(\d+)/i.exec(header);
        if (!match) {
            log("stderr", `Invalid LSP header: ${header}`);
            stdoutBuffer = stdoutBuffer.slice(headerEnd + 4);
            continue;
        }
        const start = headerEnd + 4;
        const end = start + Number(match[1]);
        if (stdoutBuffer.length < end) {
            return;
        }
        const message = JSON.parse(decoder.decode(stdoutBuffer.subarray(start, end)));
        stdoutBuffer = stdoutBuffer.slice(end);
        if (initializeRequestId !== null && message.id === initializeRequestId && message.method === undefined) {
            status("ready", message.result?.serverInfo?.version);
        }
        self.postMessage(message);
    }
}

function terminate() {
    for (const worker of threads.values()) {
        worker.terminate();
    }
    threads.clear();
    stdin.close();
}

function onThreadMessage(event) {
    const message = event.data;
    switch (message.type) {
        case "stdout":
            readStdout(message.data);
            break;
        case "stderr": {
            const text = decoder.decode(message.data);
            log("stderr", text);
            break;
        }
        case "spawn":
            spawnThread(message.tid, message.startArg);
            break;
        case "stdin-drained":
            stdin.flush();
            break;
        case "thread-exit":
            threads.delete(message.tid);
            break;
        case "exit":
            status(message.code === 0 ? "exited" : "error", `Server exited with code ${message.code}`);
            terminate();
            break;
        case "crash":
            status("error", `Server crashed: ${message.message}`);
            terminate();
            break;
    }
}

function spawnThread(tid, startArg) {
    const worker = new Worker(new URL("./wasi-thread.js", import.meta.url), { type: "module" });
    worker.addEventListener("message", onThreadMessage);
    worker.addEventListener("error", (event) => {
        status("error", `Thread worker failed: ${event.message}`);
        terminate();
    });
    threads.set(tid ?? 0, worker);
    worker.postMessage({ ...serverProcess, tid, startArg });
}

async function fetchWithProgress(url) {
    const response = await fetch(url);
    if (!response.ok) {
        throw new Error(`Failed to download ${url}: ${response.status} ${response.statusText}`);
    }
    const reader = response.body.getReader();
    const chunks = [];
    let received = 0;
    for (;;) {
        const { done, value } = await reader.read();
        if (done) {
            break;
        }
        chunks.push(value);
        received += value.length;
        status("loading", `Downloading server (${(received / 1048576).toFixed(1)} MB)`);
    }
    const bytes = new Uint8Array(received);
    let offset = 0;
    for (const chunk of chunks) {
        bytes.set(chunk, offset);
        offset += chunk.length;
    }
    return bytes.buffer;
}

async function start({ wasmUrl, args, env }) {
    try {
        if (!self.crossOriginIsolated) {
            throw new Error("This page is not cross origin isolated, SharedArrayBuffer is unavailable.");
        }
        const bytes = await fetchWithProgress(wasmUrl);
        status("loading", "Compiling server");
        const limits = readImportedMemoryLimits(bytes);
        if (limits === null) {
            throw new Error("Server module does not import its memory, is it built for wasm32-wasip1-threads ?");
        }
        const module = await WebAssembly.compile(bytes);
        const memory = new WebAssembly.Memory({ initial: limits.initial, maximum: limits.maximum, shared: true });
        serverProcess = { module, memory, args, env, stdinBuffer, controlBuffer };
        status("loading", "Starting server");
        spawnThread(undefined, undefined);
    } catch (error) {
        status("error", String(error.message ?? error));
    }
}

self.addEventListener("message", (event) => {
    const message = event.data;
    if (message?.type === "start") {
        start(message);
    } else if (message?.jsonrpc) {
        writeMessage(message);
    }
});
