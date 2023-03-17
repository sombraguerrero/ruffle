/**
 * Conditional ruffle loader
 */

import {
    bulkMemory,
    simd,
    saturatedFloatToInt,
    signExtensions,
    referenceTypes,
} from "wasm-feature-detect";
import { setPolyfillsOnLoad } from "./js-polyfills";
import { publicPath } from "./public-path";
import type { DataLoadOptions, URLLoadOptions } from "./load-options";

declare global {
    let __webpack_public_path__: string;
}

type ProgressCallback = (bytesLoaded: number, bytesTotal: number) => void;

/**
 * Load ruffle from an automatically-detected location.
 *
 * This function returns a new instance of Ruffle and downloads it every time.
 * You should not use it directly; this module will memoize the resource
 * download.
 *
 * @param config The `window.RufflePlayer.config` object.
 * @param progressCallback The callback that will be run with Ruffle's download progress.
 * @returns A ruffle constructor that may be used to create new Ruffle
 * instances.
 */
async function fetchRuffle(
    config: URLLoadOptions | DataLoadOptions | object,
    progressCallback?: ProgressCallback
): Promise<typeof Ruffle> {
    // Apply some pure JavaScript polyfills to prevent conflicts with external
    // libraries, if needed.
    setPolyfillsOnLoad();

    // NOTE: Keep this list in sync with $RUSTFLAGS in the CI build config!
    const extensionsSupported: boolean = (
        await Promise.all([
            bulkMemory(),
            simd(),
            saturatedFloatToInt(),
            signExtensions(),
            referenceTypes(),
        ])
    ).every(Boolean);

    if (!extensionsSupported) {
        console.log(
            "Some WebAssembly extensions are NOT available, falling back to the vanilla WebAssembly module"
        );
    }

    __webpack_public_path__ = publicPath(config);

    // Note: The argument passed to import() has to be a simple string literal,
    // otherwise some bundler will get confused and won't include the module?
    const { default: init, Ruffle } = await (extensionsSupported
        ? import("../dist/ruffle_web-wasm_extensions")
        : import("../dist/ruffle_web"));
    let response;
    const wasmUrl = extensionsSupported
        ? new URL("../dist/ruffle_web-wasm_extensions_bg.wasm", import.meta.url)
        : new URL("../dist/ruffle_web_bg.wasm", import.meta.url);
    const wasmResponse = await fetch(wasmUrl);
    if (progressCallback) {
        const contentLength = wasmResponse.headers.get("content-length") || "";
        let bytesLoaded = 0;
        // Use parseInt rather than Number so the empty string is coerced to NaN instead of 0
        const bytesTotal = parseInt(contentLength);
        response = new Response(
            new ReadableStream({
                async start(controller) {
                    const reader = wasmResponse.body?.getReader();
                    if (!reader) {
                        throw "Response had no body";
                    }
                    progressCallback(bytesLoaded, bytesTotal);
                    for (;;) {
                        const { done, value } = await reader.read();
                        if (done) {
                            break;
                        }
                        if (value?.byteLength) {
                            bytesLoaded += value?.byteLength;
                        }
                        controller.enqueue(value);
                        progressCallback(bytesLoaded, bytesTotal);
                    }
                    controller.close();
                },
            }),
            wasmResponse
        );
    } else {
        response = wasmResponse;
    }

    await init(response);

    return Ruffle;
}

type Ruffle =
    | typeof import("../dist/ruffle_web")["Ruffle"]
    | typeof import("../dist/ruffle_web-wasm_extensions")["Ruffle"];

let lastLoaded: Promise<Ruffle> | null = null;

/**
 * Obtain an instance of `Ruffle`.
 *
 * This function returns a promise which yields `Ruffle` asynchronously.
 *
 * @param config The `window.RufflePlayer.config` object.
 * @param progressCallback The callback that will be run with Ruffle's download progress.
 * @returns A ruffle constructor that may be used to create new Ruffle
 * instances.
 */
export function loadRuffle(
    config: URLLoadOptions | DataLoadOptions | object,
    progressCallback?: ProgressCallback
): Promise<Ruffle> {
    if (lastLoaded === null) {
        lastLoaded = fetchRuffle(config, progressCallback);
    }

    return lastLoaded;
}
