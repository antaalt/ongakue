// Worker running a single wasm thread of the server. The main thread runs `_start`, others run `wasi_thread_start`.
import { createWasiImports, WasiExit } from "./wasi.js";

self.addEventListener("message", async (event) => {
    const { module, memory, args, env, stdinBuffer, controlBuffer, tid, startArg } = event.data;
    const post = (message, transfer) => self.postMessage(message, transfer);
    try {
        const imports = createWasiImports({ memory, args, env, stdinBuffer, controlBuffer, post });
        const instance = await WebAssembly.instantiate(module, imports);
        if (tid === undefined) {
            instance.exports._start();
            post({ type: "exit", code: 0 });
        } else {
            instance.exports.wasi_thread_start(tid, startArg);
            post({ type: "thread-exit", tid });
        }
    } catch (error) {
        if (error instanceof WasiExit) {
            post({ type: "exit", code: error.code });
        } else {
            // A trap in any thread (panic, stack overflow...) leaves the shared memory in an unknown state.
            post({ type: "crash", tid, message: String(error && error.stack || error) });
        }
    }
    self.close();
}, { once: true });
