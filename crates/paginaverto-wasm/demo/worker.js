// paginaverto-wasm demo: the reading runs here, off the page's thread.
// Messages in: { type: "init", pkg } then { type: "read", bytes }.
// Messages out: { ok: true, ... } or { ok: false, kind, message }.
let wasm = null;

self.onmessage = async (e) => {
  const msg = e.data;
  try {
    if (msg.type === "init") {
      wasm = await import(msg.pkg);
      await wasm.default();
      self.postMessage({ ok: true, version: wasm.version() });
    } else if (msg.type === "read") {
      const book = wasm.read(msg.bytes);
      const figures = book.chapters.flatMap((c) => c.blocks.filter((b) => b.kind === "figure").map((b) => b.data.buffer));
      self.postMessage({ ok: true, book }, figures);
    }
  } catch (err) {
    // A thrown paginaverto error carries `kind`; a trap (RuntimeError) does not.
    self.postMessage({ ok: false, kind: err?.kind ?? "Internal", message: String(err?.message ?? err) });
  }
};
