# Third-Party Notices

Conclave's workspace memory system adapts concepts and schema from open-source
software and bundles open-source components. This file records the required
notices. It ships inside the application bundle.

## MemPalace (adapted logic and schema)

The workspace memory system's storage model (float32 vector BLOBs in SQLite,
per-collection model/dimension identity, exact cosine top-k search) is a Rust
reimplementation adapted from MemPalace's `sqlite_exact` backend and embedder
identity invariants. The implementation was written for Conclave and modified
from the original; it is not a copy of MemPalace source files.

- Upstream: <https://github.com/MemPalace/mempalace>
- Source commit consulted for the port: `da5a48caf5d8a843df7568a00e44c714bd91ab11`
- Adapted areas: `src/engine/migrations/0009_memory_system.sql`,
  `src/engine/repo/memory.rs`, `src/engine/runtime/vec_codec.rs`,
  `src/engine/commands/memory.rs` (exact-search shape)

```
MIT License

Copyright (c) 2026 MemPalace Contributors

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## fastembed-rs (linked library)

Text embedding runtime used by the memory system.

- Upstream: <https://github.com/Anush008/fastembed-rs>
- License: Apache-2.0 (<https://www.apache.org/licenses/LICENSE-2.0>)

## ort — ONNX Runtime bindings (linked library)

ONNX inference runtime linked statically for embedding inference. `ort` is
dual-licensed MIT OR Apache-2.0; ONNX Runtime itself is MIT.

- Upstream: <https://github.com/pykeio/ort> · <https://github.com/microsoft/onnxruntime>
- License: MIT OR Apache-2.0 (`ort`); MIT (ONNX Runtime)

## all-MiniLM-L6-v2 (embedding model, downloaded at first use)

Quantized ONNX export of `sentence-transformers/all-MiniLM-L6-v2`, downloaded
once to the local model cache on first use; not distributed inside this bundle.

- Model: <https://huggingface.co/Xenova/all-MiniLM-L6-v2>
  (original: <https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2>)
- License: Apache-2.0
