// Runs a `wasm32-wasip1` test binary with Node's built-in WASI, so the determinism
// fingerprint (ADR 0011) can be checked locally without installing wasmtime.
//
//   cargo test -p knorke-contract-tests --test determinism --target wasm32-wasip1 --no-run
//   node xtask/wasi-run.mjs target/wasm32-wasip1/debug/deps/determinism-<hash>.wasm
//
// CI uses wasmtime (`.github/workflows/ci.yml`); this is a second, independent engine (V8).
import { readFile } from 'node:fs/promises';
import process from 'node:process';
import { WASI } from 'node:wasi';

const file = process.argv[2];
if (!file) {
  console.error('usage: node xtask/wasi-run.mjs <test.wasm> [test args]');
  process.exit(2);
}
process.removeAllListeners('warning'); // node:wasi is flagged experimental
const args = [file, '--nocapture', '--test-threads=1', ...process.argv.slice(3)];
const wasi = new WASI({ version: 'preview1', args, env: {} });
const module = await WebAssembly.compile(await readFile(file));
const instance = await WebAssembly.instantiate(module, wasi.getImportObject());
process.exitCode = wasi.start(instance);
