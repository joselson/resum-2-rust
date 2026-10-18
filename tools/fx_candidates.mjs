// Candidate default values per FX module parameter, from factory presets plus round numbers.
// usage: node tools/fx_candidates.mjs PRESET_DIR > fx-candidates.json
import { readFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
const { decodePreset } = await import(new URL("../../serumpreset.mjs", import.meta.url));
const order = JSON.parse(readFileSync(new URL("../data/serum2-param-order.json", import.meta.url)));
const files = execFileSync("find", [process.argv[2], "-iname", "*.SerumPreset"], { encoding: "utf8" }).trim().split("\n");
const num = (v) => (typeof v === "number" ? v : v?.$float);
const seen = {}; // module -> param -> {numbers: Map(value->count), texts: Set}
const types = {};
for (const f of files) {
  const p = decodePreset(readFileSync(f)).payload;
  for (let r = 0; r < 3; r++) {
    for (const m of p[`FXRack${r}`]?.FX ?? []) {
      const name = Object.keys(m).find((k) => k.startsWith("FX"));
      types[name] = m.type;
      const pp = m[name].plainParams;
      if (!pp || pp === "default") continue;
      for (const [k, v] of Object.entries(pp)) {
        const e = ((seen[name] ??= {})[k] ??= { numbers: new Map(), texts: new Set() });
        const x = num(v);
        if (typeof x === "number") e.numbers.set(x, (e.numbers.get(x) ?? 0) + 1); else if (typeof v === "string") e.texts.add(v);
      }
    }
  }
}
const round = [0, 0.001, 0.01, 0.1, 0.25, 0.5, 0.75, 1, 2, 3, 4, 5, 6, 8, 10, 12, 15, 16, 20, 24, 25, 30, 33.3, 40, 50, 60, 66.6, 70, 75, 80, 90, 100, 120, 200, 250, 300, 400, 440, 500, 1000, 2000, 5000, 10000, 20000, -100, -50, -24, -12, -6, -1];
const out = {};
for (const [name, type] of Object.entries(types)) {
  const params = new Set([...(order[name] ?? []), ...Object.keys(seen[name] ?? {})]);
  out[name] = { type, params: {} };
  for (const k of params) {
    const e = seen[name]?.[k];
    const observed = e ? [...e.numbers.entries()].sort((a, b) => b[1] - a[1]).map(([v]) => v) : [];
    // Mid-range values between observed extremes also make good candidates.
    const lo = observed.length ? Math.min(...observed) : null, hi = observed.length ? Math.max(...observed) : null;
    const mids = lo !== null && hi !== lo ? [0.25, 0.5, 0.75].map((t) => lo + (hi - lo) * t) : [];
    const values = [...new Set([...round, ...observed.slice(0, 40), ...mids, ...(lo !== null ? [lo, hi] : [])])];
    out[name].params[k] = { values, texts: e ? [...e.texts] : [], observed_min: lo, observed_max: hi, stored: e ? [...e.numbers.values()].reduce((a, b) => a + b, 0) + e.texts.size : 0 };
  }
}
console.log(JSON.stringify(out));
