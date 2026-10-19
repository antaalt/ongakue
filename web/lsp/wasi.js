// Minimal WASI preview1 host with wasi-threads support, just enough to run shader-language-server in a browser.
//
// The server is built for wasm32-wasip1-threads, so every wasm thread (including the main one) runs inside its own
// worker, sharing a single WebAssembly.Memory. Blocking calls (stdin reads, sleeps) rely on Atomics.wait, which is
// allowed in workers only. There is no file system: the documents are sent through LSP, so no preopen is exposed.

const ERRNO_SUCCESS = 0;
const ERRNO_BADF = 8;
const ERRNO_INVAL = 28;
const ERRNO_NOENT = 44;
const ERRNO_NOTSUP = 58;
const ERRNO_SPIPE = 70;

const FILETYPE_CHARACTER_DEVICE = 2;
const CLOCK_REALTIME = 0;
const EVENTTYPE_CLOCK = 0;
const EVENTTYPE_FD_READ = 1;
const SUBCLOCKFLAGS_ABSTIME = 1;

// Layout of the control buffer shared between all threads & the coordinator.
export const CONTROL_NEXT_TID = 0;
export const CONTROL_SIZE = 4;

// Layout of the stdin ring buffer. The coordinator is the only producer and the server reader thread the only consumer.
const STDIN_READ = 0; // Index of the next byte to read.
const STDIN_WRITE = 1; // Index of the next byte to write.
const STDIN_COUNT = 2; // Bytes available to read, the consumer waits on it.
const STDIN_PRODUCER_WAITING = 3; // Set by the producer when the ring is full, so the consumer warns it once drained.
const STDIN_CLOSED = 4;
const STDIN_HEADER_BYTES = 8 * Int32Array.BYTES_PER_ELEMENT;
export const STDIN_CAPACITY = 1 << 20;

export class WasiExit {
    constructor(code) {
        this.code = code;
    }
}

export function createStdinBuffer() {
    return new SharedArrayBuffer(STDIN_HEADER_BYTES + STDIN_CAPACITY);
}

// Producer side of the stdin ring buffer, owned by the coordinator which must never block.
export class StdinWriter {
    constructor(buffer) {
        this.header = new Int32Array(buffer, 0, 8);
        this.data = new Uint8Array(buffer, STDIN_HEADER_BYTES, STDIN_CAPACITY);
        this.pending = [];
    }
    write(bytes) {
        this.pending.push(bytes);
        this.flush();
    }
    // Push as much pending data as the ring can hold. Called again when the consumer reports it drained the ring.
    flush() {
        while (this.pending.length > 0) {
            const free = STDIN_CAPACITY - Atomics.load(this.header, STDIN_COUNT);
            if (free === 0) {
                Atomics.store(this.header, STDIN_PRODUCER_WAITING, 1);
                return;
            }
            const chunk = this.pending[0];
            const size = Math.min(free, chunk.length);
            let write = this.header[STDIN_WRITE];
            const first = Math.min(size, STDIN_CAPACITY - write);
            this.data.set(chunk.subarray(0, first), write);
            this.data.set(chunk.subarray(first, size), 0);
            write = (write + size) % STDIN_CAPACITY;
            Atomics.store(this.header, STDIN_WRITE, write);
            Atomics.add(this.header, STDIN_COUNT, size);
            Atomics.notify(this.header, STDIN_COUNT);
            if (size === chunk.length) {
                this.pending.shift();
            } else {
                this.pending[0] = chunk.subarray(size);
            }
        }
    }
    close() {
        Atomics.store(this.header, STDIN_CLOSED, 1);
        Atomics.notify(this.header, STDIN_COUNT);
    }
}

// Consumer side of the stdin ring buffer, used from wasm threads which are allowed to block.
class StdinReader {
    constructor(buffer, onDrained) {
        this.header = new Int32Array(buffer, 0, 8);
        this.data = new Uint8Array(buffer, STDIN_HEADER_BYTES, STDIN_CAPACITY);
        this.onDrained = onDrained;
    }
    // Wait until some data is available or the timeout (ms) expires. Return the available byte count.
    wait(timeout = Infinity) {
        let available = Atomics.load(this.header, STDIN_COUNT);
        const deadline = performance.now() + timeout;
        while (available === 0 && Atomics.load(this.header, STDIN_CLOSED) === 0) {
            const remaining = deadline - performance.now();
            if (remaining <= 0) {
                break;
            }
            Atomics.wait(this.header, STDIN_COUNT, 0, remaining === Infinity ? undefined : remaining);
            available = Atomics.load(this.header, STDIN_COUNT);
        }
        return available;
    }
    read(target) {
        const available = this.wait();
        const size = Math.min(available, target.length);
        if (size === 0) {
            return 0; // EOF
        }
        const read = this.header[STDIN_READ];
        const first = Math.min(size, STDIN_CAPACITY - read);
        target.set(this.data.subarray(read, read + first), 0);
        target.set(this.data.subarray(0, size - first), first);
        Atomics.store(this.header, STDIN_READ, (read + size) % STDIN_CAPACITY);
        Atomics.sub(this.header, STDIN_COUNT, size);
        if (Atomics.exchange(this.header, STDIN_PRODUCER_WAITING, 0) === 1) {
            this.onDrained();
        }
        return size;
    }
}

function encodeStrings(strings) {
    const encoder = new TextEncoder();
    return strings.map(value => encoder.encode(value + "\0"));
}

// Create the import object for one wasm thread.
// `post` sends a message to the coordinator: stdout / stderr writes, thread spawn requests & stdin drain notifications.
export function createWasiImports({ memory, args, env, stdinBuffer, controlBuffer, post }) {
    const argv = encodeStrings(args);
    const environ = encodeStrings(Object.entries(env).map(([key, value]) => `${key}=${value}`));
    const control = new Int32Array(controlBuffer);
    const stdin = new StdinReader(stdinBuffer, () => post({ type: "stdin-drained" }));
    const sleepCell = new Int32Array(new SharedArrayBuffer(4));

    // Shared memory may grow from any thread, so views are always recreated from the current buffer.
    const view = () => new DataView(memory.buffer);
    const bytes = (ptr, len) => new Uint8Array(memory.buffer, ptr, len);
    // Most web APIs (TextDecoder, crypto, postMessage transfer) refuse views on shared memory, so copy out.
    const copyOut = (ptr, len) => bytes(ptr, len).slice();

    const now = (clockId) => {
        if (clockId === CLOCK_REALTIME) {
            return BigInt(Date.now()) * 1000000n;
        }
        return BigInt(Math.round((performance.timeOrigin + performance.now()) * 1000000));
    };
    const writeStrings = (strings, pointers, buffer) => {
        const dv = view();
        for (const value of strings) {
            dv.setUint32(pointers, buffer, true);
            bytes(buffer, value.length).set(value);
            pointers += 4;
            buffer += value.length;
        }
        return ERRNO_SUCCESS;
    };
    const writeSizes = (strings, countOut, sizeOut) => {
        const dv = view();
        dv.setUint32(countOut, strings.length, true);
        dv.setUint32(sizeOut, strings.reduce((size, value) => size + value.length, 0), true);
        return ERRNO_SUCCESS;
    };
    const iovecs = (ptr, len) => {
        const dv = view();
        const result = [];
        for (let i = 0; i < len; i++) {
            result.push({ buf: dv.getUint32(ptr + i * 8, true), len: dv.getUint32(ptr + i * 8 + 4, true) });
        }
        return result;
    };

    const wasi = {
        args_get: (argvPtr, argvBuf) => writeStrings(argv, argvPtr, argvBuf),
        args_sizes_get: (countOut, sizeOut) => writeSizes(argv, countOut, sizeOut),
        environ_get: (environPtr, environBuf) => writeStrings(environ, environPtr, environBuf),
        environ_sizes_get: (countOut, sizeOut) => writeSizes(environ, countOut, sizeOut),
        clock_res_get: (_clockId, out) => {
            view().setBigUint64(out, 1000n, true);
            return ERRNO_SUCCESS;
        },
        clock_time_get: (clockId, _precision, out) => {
            view().setBigUint64(out, now(clockId), true);
            return ERRNO_SUCCESS;
        },
        random_get: (ptr, len) => {
            const random = new Uint8Array(len);
            for (let offset = 0; offset < len; offset += 65536) {
                crypto.getRandomValues(random.subarray(offset, Math.min(len, offset + 65536)));
            }
            bytes(ptr, len).set(random);
            return ERRNO_SUCCESS;
        },
        fd_read: (fd, iovs, iovsLen, nreadOut) => {
            if (fd !== 0) {
                return ERRNO_BADF;
            }
            // Read in the first non empty buffer only, so that we never block once some data was read.
            let nread = 0;
            for (const iov of iovecs(iovs, iovsLen)) {
                if (iov.len > 0) {
                    const target = new Uint8Array(iov.len);
                    nread = stdin.read(target);
                    bytes(iov.buf, nread).set(target.subarray(0, nread));
                    break;
                }
            }
            view().setUint32(nreadOut, nread, true);
            return ERRNO_SUCCESS;
        },
        fd_write: (fd, iovs, iovsLen, nwrittenOut) => {
            if (fd !== 1 && fd !== 2) {
                return ERRNO_BADF;
            }
            const chunks = iovecs(iovs, iovsLen).filter(iov => iov.len > 0).map(iov => copyOut(iov.buf, iov.len));
            const size = chunks.reduce((total, chunk) => total + chunk.length, 0);
            const data = new Uint8Array(size);
            let offset = 0;
            for (const chunk of chunks) {
                data.set(chunk, offset);
                offset += chunk.length;
            }
            post({ type: fd === 1 ? "stdout" : "stderr", data }, [data.buffer]);
            view().setUint32(nwrittenOut, size, true);
            return ERRNO_SUCCESS;
        },
        fd_fdstat_get: (fd, out) => {
            if (fd > 2) {
                return ERRNO_BADF;
            }
            const dv = view();
            dv.setUint8(out, FILETYPE_CHARACTER_DEVICE);
            dv.setUint16(out + 2, 0, true);
            dv.setBigUint64(out + 8, 0xFFFFFFFFFFFFFFFFn, true);
            dv.setBigUint64(out + 16, 0xFFFFFFFFFFFFFFFFn, true);
            return ERRNO_SUCCESS;
        },
        fd_fdstat_set_flags: (fd) => fd > 2 ? ERRNO_BADF : ERRNO_SUCCESS,
        fd_filestat_get: (fd, out) => {
            if (fd > 2) {
                return ERRNO_BADF;
            }
            bytes(out, 64).fill(0);
            view().setUint8(out + 16, FILETYPE_CHARACTER_DEVICE);
            return ERRNO_SUCCESS;
        },
        fd_close: (fd) => fd > 2 ? ERRNO_BADF : ERRNO_SUCCESS,
        fd_seek: (fd) => fd > 2 ? ERRNO_BADF : ERRNO_SPIPE,
        // No preopened directory, libc stops scanning preopens on the first BADF.
        fd_prestat_get: () => ERRNO_BADF,
        fd_prestat_dir_name: () => ERRNO_BADF,
        path_open: () => ERRNO_NOENT,
        path_filestat_get: () => ERRNO_NOENT,
        path_readlink: () => ERRNO_NOENT,
        sock_accept: () => ERRNO_NOTSUP,
        poll_oneoff: (subscriptions, events, count, neventsOut) => {
            const dv = view();
            let timeout = Infinity;
            let timer = null;
            const readers = [];
            for (let i = 0; i < count; i++) {
                const sub = subscriptions + i * 48;
                const userdata = dv.getBigUint64(sub, true);
                const tag = dv.getUint8(sub + 8);
                if (tag === EVENTTYPE_CLOCK) {
                    const clockId = dv.getUint32(sub + 16, true);
                    let delay = dv.getBigUint64(sub + 24, true);
                    if (dv.getUint16(sub + 40, true) & SUBCLOCKFLAGS_ABSTIME) {
                        delay = delay > now(clockId) ? delay - now(clockId) : 0n;
                    }
                    const ms = Number(delay) / 1000000;
                    if (ms < timeout) {
                        timeout = ms;
                        timer = userdata;
                    }
                } else if (tag === EVENTTYPE_FD_READ && dv.getUint32(sub + 16, true) === 0) {
                    readers.push(userdata);
                } else {
                    return ERRNO_NOTSUP;
                }
            }
            const ready = [];
            if (readers.length > 0) {
                const available = stdin.wait(timeout);
                if (available > 0) {
                    readers.forEach(userdata => ready.push({ userdata, type: EVENTTYPE_FD_READ, nbytes: available }));
                }
            } else if (timeout !== Infinity) {
                Atomics.wait(sleepCell, 0, 0, timeout);
            }
            if (ready.length === 0 && timer !== null) {
                ready.push({ userdata: timer, type: EVENTTYPE_CLOCK, nbytes: 0 });
            }
            ready.forEach((event, i) => {
                const out = events + i * 32;
                bytes(out, 32).fill(0);
                dv.setBigUint64(out, event.userdata, true);
                dv.setUint8(out + 10, event.type);
                dv.setBigUint64(out + 16, BigInt(event.nbytes), true);
            });
            dv.setUint32(neventsOut, ready.length, true);
            return ERRNO_SUCCESS;
        },
        sched_yield: () => ERRNO_SUCCESS,
        proc_exit: (code) => {
            throw new WasiExit(code);
        },
    };
    // Unknown imports should not prevent the module from loading, only fail when called.
    const wasiProxy = new Proxy(wasi, {
        get: (target, name) => target[name] ?? (() => ERRNO_NOTSUP),
    });
    return {
        env: { memory },
        wasi_snapshot_preview1: wasiProxy,
        wasi: {
            "thread-spawn": (startArg) => {
                const tid = Atomics.add(control, CONTROL_NEXT_TID, 1) + 1;
                if (tid > 0x1FFFFFFF) {
                    return -ERRNO_INVAL;
                }
                post({ type: "spawn", tid, startArg });
                return tid;
            },
        },
    };
}

function readLeb128(bytes, state) {
    let result = 0;
    let shift = 0;
    let byte;
    do {
        byte = bytes[state.offset++];
        result += (byte & 0x7F) * 2 ** shift;
        shift += 7;
    } while (byte & 0x80);
    return result;
}

// WebAssembly.Module.imports does not expose memory limits, so read them from the import section.
// An imported shared memory must match the limits declared by the module.
export function readImportedMemoryLimits(buffer) {
    const bytes = new Uint8Array(buffer);
    const state = { offset: 8 }; // Skip magic & version.
    const skipName = () => {
        const length = readLeb128(bytes, state);
        state.offset += length;
    };
    const readLimits = () => {
        const flags = bytes[state.offset++];
        const initial = readLeb128(bytes, state);
        const maximum = (flags & 1) ? readLeb128(bytes, state) : undefined;
        return { initial, maximum, shared: (flags & 2) !== 0 };
    };
    while (state.offset < bytes.length) {
        const id = bytes[state.offset++];
        const size = readLeb128(bytes, state);
        const end = state.offset + size;
        if (id === 2) {
            const count = readLeb128(bytes, state);
            for (let i = 0; i < count; i++) {
                skipName();
                skipName();
                const kind = bytes[state.offset++];
                if (kind === 0) {
                    readLeb128(bytes, state);
                } else if (kind === 1) {
                    state.offset++;
                    readLimits();
                } else if (kind === 2) {
                    return readLimits();
                } else if (kind === 3) {
                    state.offset += 2;
                } else if (kind === 4) {
                    state.offset++;
                    readLeb128(bytes, state);
                }
            }
            return null;
        }
        state.offset = end;
    }
    return null;
}
