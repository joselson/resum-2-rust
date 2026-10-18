// Lists plainParams keys stored in presets that the generated tables lack, with observed values.
// usage: node tools/preset_keys.mjs PRESET_DIR > data/serum2-preset-keys.json
import { readFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
const { decodePreset } = await import(new URL("../../serumpreset.mjs", import.meta.url));
const known = new Set();
const rs = readFileSync(new URL("../crates/engine/src/params/serum2.rs", import.meta.url), "utf8");
let module = null;
for (const line of rs.split("\n")) {
  const m = line.match(/^\s+name: "(\w+)",$/); if (m) module = m[1];
  // Only parameters with independent evidence count as known; ones that came from a
  // previous preset scan (or are unobserved placeholders) are re-derived each run.
  const p = line.match(/ParamSpec \{ name: "(\w+)".*evidence: Evidence::(\w+)/);
  if (p && module && !["Presets", "Unknown"].includes(p[2])) known.add(`${module}.${p[1]}`);
}
const sections = [["Oscillator#.plainParams", "Oscillator"], ["Oscillator#.WTOsc#.plainParams", "WTOsc"], ["Oscillator#.SampleOsc#.plainParams", "SampleOsc"], ["Oscillator#.MultiSampleOsc#.plainParams", "MultiSampleOsc"], ["Oscillator#.GranularOsc#.plainParams", "GranularOsc"], ["Oscillator#.SpectralOsc#.plainParams", "SpectralOsc"], ["Oscillator#.NoiseOsc#.plainParams", "NoiseOsc"], ["Oscillator#.SubOsc#.plainParams", "SubOsc"], ["VoiceFilter#.plainParams", "VoiceFilter"], ["Env#.plainParams", "Env"], ["LFO#.plainParams", "LFO"], ["Macro#.plainParams", "Macro"], ["Global#.plainParams", "Global"], ["RoutingSlot#.plainParams", "RoutingSlot"], ["VoicePanel#.plainParams", "VoicePanel"], ["ModSlot#.plainParams", "ModSlot"], ["Arp#.plainParams", "Arp"], ["ClipPlayer#.plainParams", "ClipPlayer"], ["PitchQuantizer#.plainParams", "PitchQuantizer"]];
const files = execFileSync("find", [process.argv[2], "-iname", "*.SerumPreset"], { encoding: "utf8" }).trim().split("\n");
const found = {};
const num = (v) => typeof v === "number" ? v : v?.$float;
for (const f of files) {
  const p = decodePreset(readFileSync(f)).payload;
  for (const [pat, mod] of sections) {
    for (let i = 0; i < 64; i++) {
      const path = pat.replaceAll("#", i).split(".");
      const pp = path.reduce((v, k) => v?.[k], p);
      if (!pp || typeof pp !== "object") continue;
      for (const [k, v] of Object.entries(pp)) {
        const id = `${mod}.${k}`; if (known.has(id)) continue;
        const e = (found[id] ??= { module: mod, name: k, count: 0, numbers: [], texts: [] });
        e.count++;
        const x = num(v);
        if (typeof x === "number") { if (e.numbers.length < 4000) e.numbers.push(x); } else if (typeof v === "string" && !e.texts.includes(v)) e.texts.push(v);
      }
    }
  }
}
const out = Object.values(found).map((e) => ({ module: e.module, name: e.name, count: e.count, min: e.numbers.length ? Math.min(...e.numbers) : null, max: e.numbers.length ? Math.max(...e.numbers) : null, integer: e.numbers.every((x) => Number.isInteger(x)), texts: e.texts }));
out.sort((a, b) => b.count - a.count);
console.log(JSON.stringify(out, null, 1));
