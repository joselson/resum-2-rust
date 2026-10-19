// Generates calibration probe presets: Serum 2's Init state with one change each.
// usage: node tools/make_probes.mjs INIT_PROCESSOR_STATE OUT_DIR [NAME_REGEX]
//
// INIT_PROCESSOR_STATE is the decoded IComponent block of a fresh Serum 2
// instance (an XferJson container). Each probe isolates one behaviour so a
// render comparison points at a single mapping.
import { readFileSync, writeFileSync, mkdirSync, readdirSync, unlinkSync } from "node:fs";
import { join } from "node:path";

const { decodePreset, encodePreset } = await import(new URL("../../serumpreset.mjs", import.meta.url));
const [stateFile, outDir, only] = process.argv.slice(2);
const selected = only ? new RegExp(only) : null;
const init = decodePreset(readFileSync(stateFile));
const f = (v) => (Number.isInteger(v) ? { $float: v } : v);

/** Sets section.plainParams.key for each [path, key, value]; path like "Oscillator0.WTOsc0". */
function probe(changes) {
  const payload = structuredClone(init.payload);
  delete payload.component;
  for (const change of changes) {
    if (change[0] === "__section") {
      // Replace a whole top-level section (e.g. an oscillator copied from a factory preset).
      payload[change[1]] = structuredClone(change[2]);
      continue;
    }
    if (change[0] === "__rack0") {
      payload.FXRack0 = { FX: change[1].map(([name, type, params, extra]) => ({ [name]: { plainParams: Object.keys(params).length ? Object.fromEntries(Object.entries(params).map(([k, v]) => [k, typeof v === "string" ? v : f(v)])) : "default", ...extra }, type })), displayName: "", plainParams: "default" };
      continue;
    }
    const [path, key, value] = change;
    const section = path.split(".").reduce((s, k) => (s[k] ??= {}), payload);
    if (typeof section.plainParams !== "object") section.plainParams = {};
    section.plainParams[key] = typeof value === "string" ? value : f(value);
  }
  return payload;
}

/** Replaces the main FX rack with `modules`: [[name, type, {param: value}], ...]. */
const rack = (...modules) => [["__rack0", modules]];
const FX_TYPES = { FXDistortion: 0, FXFlanger: 1, FXPhaser: 2, FXChorus: 3, FXDelay: 4, FXComp: 5, FXReverb: 6, FXEQ: 7, FXFilter: 8, FXHyperD: 9, FXBode: 10, FXConv: 11, FXUtils: 12, FXSplit: 13, FXSplit3: 14, FXSplitMS: 15 };
const fx = (name, params = {}, extra = {}) => [name, FX_TYPES[name], params, extra];

const WARP_BASE = ["kSync", "kBendPos", "kBendNeg", "kBendPosNeg", "kPWM", "kASYMPos", "kASYMNeg", "kASYMPosNeg", "kFlip", "kDLM", "kRemap_1", "kRemap_2", "kRemap_3", "kRemap_4", "kQuantize", "kEvenOdd", "kFilterLPF", "kFilterHPF", "kSelfPD"];
const WARP_DIST = ["kDistTube", "kDistSoftClip", "kDistHardClip", "kDistDiode1", "kDistDiode2", "kDistLinFold", "kDistSinFold", "kDistZeroSquare", "kDistAsym", "kDistRectify", "kDistSineShaper", "kDistStompBox", "kDistTapeSat", "kDistSoftSat"];
const warp = (mode, amount) => [["Oscillator0.WTOsc0", "kParamWarpMenu", mode], ["Oscillator0.WTOsc0", "kParamWarp", amount]];

// Oscillator sections borrowed from factory presets (their sample references only).
const LIB = process.env.LUMEN_PRESET_LIBRARY;
const borrow = (preset, section) => {
  if (!LIB) return null;
  const path = join(LIB, "Presets", "Factory", preset);
  try { return decodePreset(readFileSync(path)).payload[section]; } catch { return null; }
};
const sampleOsc = borrow("Arp/ARP - Kalimba Arps.SerumPreset", "Oscillator2");
const multiOsc = borrow("Arp/ARP - Cascading Etheric Paths.SerumPreset", "Oscillator0");
const kalimbaOsc = borrow("Arp/ARP - Kalimba Arps.SerumPreset", "Oscillator0");
/** Uses a borrowed oscillator as OSC A with its settings reset to defaults plus `params`. */
const asOscA = (osc, index, params = {}) => {
  if (!osc) return [];
  const o = structuredClone(osc);
  for (const k of Object.keys(o)) if (/\d$/.test(k)) { o[k.replace(/\d$/, "0")] = o[k]; if (!k.endsWith("0")) delete o[k]; }
  o.plainParams = { kParamType: index === "sample" ? "kOsc_Sample" : "kOsc_MultiSample", ...params };
  return [["__section", "Oscillator0", o]];
};

const FILTER_TYPES = JSON.parse(readFileSync(new URL("../data/serum2-choices.json", import.meta.url), "utf8"))["VoiceFilter0.plainParams.kParamType"].options.map((o) => o[1]);
// Most used in the factory presets (voice + FX filters), and types whose Var control changes the response.
const FILTER_TOP = ["MgL6", "MgL12", "MgL18", "MgL24", "LadderEMS", "LadderMg", "LadderAcid", "DirtyMg", "L12", "L18", "L24", "H12", "H18", "H24", "B12", "B24", "LH12", "HP12", "Diffuser", "CombP", "Reverb1", "Wsp", "PZ_SVF", "Exp", "ExpBPF", "BandReject", "Phase24P", "Combs", "Allpasses", "RM", "Comb2", "FormantONE"];
const FILTER_VAR = ["LH12", "HP12", "LBH24", "LNH24", "BPN12", "CombP", "CombH6N", "FlangeP", "Phase24P", "Diffuser", "Reverb1", "Wsp", "RM", "FormantONE", "FormantTWO", "Exp", "ExpBPF", "LadderEMS", "DirtyMg", "PZ_SVF", "Comb2", "Combs", "Allpasses", "Scream", "DJMixer", "SNH1", "LEQ12", "BEQ12"];
const FILTER_LADDERS = ["MgL6", "MgL12", "MgL18", "MgL24", "LadderMg", "LadderEMS", "LadderAcid", "DirtyMg", "ZDF_A", "Scream"];
const FILTER_SWEEP = [...FILTER_LADDERS, "L12", "L24", "B12", "N12", "P12", "N24", "P24", "H12"];
const FILTER_DELAY = ["CombP", "CombN", "CombL6P", "CombL6N", "CombH6P", "CombH6N", "CombHL6P", "CombHL6N", "FlangeP", "FlangeN", "FlangeL6P", "FlangeL6N", "FlangeH6P", "FlangeH6N", "FlangeHL6P", "FlangeHL6N", "Phase12P", "Phase12N", "Phase24P", "Phase24N", "Phase36P", "Phase36N", "Phase48P", "Phase48N", "Phase48L6P", "Phase48H6P", "Phase48HL6P", "FlangePhase12HL6P", "Comb2", "Combs", "Allpasses"];
// Three oscillators at full volume with one phase: peaks well above a single oscillator.
const LOUD = [0, 1, 2].flatMap((o) => [[`Oscillator${o}`, "kParamEnable", 1], [`Oscillator${o}`, "kParamVolume", 1], [`Oscillator${o}.WTOsc${o}`, "kParamRandomPhase", 0]]);
const filter = (type, freq, reso = 10) => [["VoiceFilter0", "kParamEnable", 1], ["VoiceFilter0", "kParamType", type], ["VoiceFilter0", "kParamFreq", freq], ["VoiceFilter0", "kParamReso", reso]];
const probes = {
  "init": [],
  "osc-octave-up": [["Oscillator0", "kParamOctave", 1]],
  "osc-semi-7": [["Oscillator0", "kParamPitch", 7]],
  "osc-fine-50": [["Oscillator0", "kParamFine", 50]],
  "osc-coarse-12.5": [["Oscillator0", "kParamCoarsePit", 12.5]],
  "osc-volume-0.25": [["Oscillator0", "kParamVolume", 0.25]],
  "osc-pan-left": [["Oscillator0", "kParamPan", -50]],
  "wt-pos-64": [["Oscillator0.WTOsc0", "kParamTablePos", 64]],
  "wt-pos-128": [["Oscillator0.WTOsc0", "kParamTablePos", 128]],
  "wt-pos-256": [["Oscillator0.WTOsc0", "kParamTablePos", 256]],
  "unison-4": [["Oscillator0", "kParamUnison", 4]],
  "unison-7-detune-0.5": [["Oscillator0", "kParamUnison", 7], ["Oscillator0", "kParamDetune", 0.5]],
  "unison-7-blend-0": [["Oscillator0", "kParamUnison", 7], ["Oscillator0", "kParamDetuneWid", 0]],
  "unison-7-width-0": [["Oscillator0", "kParamUnison", 7], ["Oscillator0", "kParamUnisonStereo", 0]],
  "filter-mgl12-0.3": filter("MgL12", 0.3),
  "filter-mgl12-0.5": filter("MgL12", 0.5),
  "filter-mgl12-0.7": filter("MgL12", 0.7),
  "filter-mgl12-reso-80": filter("MgL12", 0.5, 80),
  "filter-mgl24-0.5": filter("MgL24", 0.5),
  "filter-l12-0.5": filter("L12", 0.5),
  "filter-l24-0.5": filter("L24", 0.5),
  "filter-h12-0.5": filter("H12", 0.5),
  "filter-b12-0.5": filter("B12", 0.5),
  "filter-n12-0.5": filter("N12", 0.5),
  "filter-drive-80": [...filter("MgL12", 0.6), ["VoiceFilter0", "kParamDrive", 80]],
  "env-attack-0.3": [["Env0", "kParamAttack", 0.3]],
  "env-decay-0.3-sus-0.2": [["Env0", "kParamDecay", 0.3], ["Env0", "kParamSustain", 0.2]],
  "env-release-0.8": [["Env0", "kParamRelease", 0.8]],
  "env-hold-0.4-sus-0": [["Env0", "kParamHold", 0.4], ["Env0", "kParamDecay", 0.3], ["Env0", "kParamSustain", 0]],
  "env-curve-attack-90": [["Env0", "kParamAttack", 0.5], ["Env0", "kParamCurve1", 90]],
  "env-curve-decay-10": [["Env0", "kParamDecay", 0.5], ["Env0", "kParamSustain", 0], ["Env0", "kParamCurve2", 10]],
  "master-volume-0.6": [["Global0", "kParamMasterVolume", 0.6]],
  "noise-on": [["Oscillator3", "kParamEnable", 1], ["Oscillator3", "kParamVolume", 0.5], ["Oscillator0", "kParamEnable", 0]],
  "sub-sine": [["Oscillator4", "kParamEnable", 1], ["Oscillator0", "kParamEnable", 0]],
  ...Object.fromEntries(["kRoundRect", "kTriangle", "kSaw", "kSquare", "kPulse"].map((s) => [`sub-${s.slice(1).toLowerCase()}`, [["Oscillator4", "kParamEnable", 1], ["Oscillator4.SubOsc4", "kParamShape", s], ["Oscillator0", "kParamEnable", 0]]])),
  "sub-octave-down": [["Oscillator4", "kParamEnable", 1], ["Oscillator4", "kParamOctave", -1], ["Oscillator0", "kParamEnable", 0]],
  "osc-b-on": [["Oscillator1", "kParamEnable", 1]],
  // FX: each module alone at its defaults, then key parameters.
  ...Object.fromEntries(Object.keys(FX_TYPES).filter((n) => !n.startsWith("FXSplit") && n !== "FXConv").map((n) => [`fx-${n.slice(2).toLowerCase()}`, rack(fx(n))])),
  "fx-conv": rack(fx("FXConv", {}, { relativePathToIR: "Factory/Massive/Crystal Hall.flac" })),
  "fx-distortion-drive-80": rack(fx("FXDistortion", { kParamDrive: 80 })),
  "fx-distortion-softclip": rack(fx("FXDistortion", { kParamMode: "kSoftClip" })),
  "fx-delay-fb-80": rack(fx("FXDelay", { kParamFeedback: 80, kParamWet: 50 })),
  "fx-delay-free-0.1": rack(fx("FXDelay", { kParamBeatSync: 0, kParamTimeL: 0.1, kParamTimeR: 0.1, kParamWet: 100, kParamFeedback: 0 })),
  "fx-reverb-size-90": rack(fx("FXReverb", { kParamSize: 90, kParamWet: 50 })),
  "fx-reverb-wet-100": rack(fx("FXReverb", { kParamWet: 100 })),
  "fx-comp-thresh-0.2": rack(fx("FXComp", { kParamThresh: 0.2 })),
  "fx-eq-low-plus-12": rack(fx("FXEQ", { kParamGain1: 12 })),
  "fx-eq-high-minus-12": rack(fx("FXEQ", { kParamGain2: -12 })),
  "fx-eq-peak-200": rack(fx("FXEQ", { kParamType1: 1, kParamFreq1: 200, kParamGain1: 12 })),
  "fx-filter-0.3": rack(fx("FXFilter", { kParamFreq: 0.3 })),
  "fx-utils-width-0": rack(fx("FXUtils", { kParamWidth: 0 })),
  "fx-utils-level-0.25": rack(fx("FXUtils", { kParamLevelOut: 0.25 })),
  "fx-chorus-wet-100": rack(fx("FXChorus", { kParamWet: 100 })),
  "fx-hyper-dim-100": rack(fx("FXHyperD", { kParamDimEWet: 100 })),
  "fx-bode-shift-50": rack(fx("FXBode", { kParamShift: 50 })),
  ...Object.fromEntries([0, 0.1, 0.25, 0.5].map((t) => [`fx-comp-limit-${t}`, rack(fx("FXComp", { kParamThresh: t, kParamRatio: 1000000 }))])),
  "fx-comp-multiband": rack(fx("FXComp", { kParamMultiband: 1 })),
  "fx-comp-ratio-4-thresh-0.05": rack(fx("FXComp", { kParamThresh: 0.05, kParamRatio: 4 })),
  // Warps on OSC A: base modes at 0.5 and 1.0, distortions at 0.5, cross-mod from B or Sub.
  ...Object.fromEntries(WARP_BASE.flatMap((m) => [0.5, 1].map((a) => [`warp-${m.slice(1)}-${a}`, warp(m, a)]))),
  ...Object.fromEntries(WARP_DIST.map((m) => [`warp-${m.slice(1)}-0.5`, warp(m, 0.5)])),
  ...Object.fromEntries(["FM", "FMX", "FMP", "PD", "AM", "RM"].flatMap((k) => [["OSC", 1], ["SUB", 4]].map(([src, slot]) => [`warp-${k}_${src}-0.5`, [...warp(`k${k}_${src}`, 0.5), [`Oscillator${slot}`, "kParamEnable", 1], [`RoutingSlot${slot}`, "kParamRoutingDest", "kRoutingDestNone"]]]))),
  // Samples: OSC A playing a borrowed sample or multisample.
  "sample-plain": asOscA(sampleOsc, "sample"),
  "sample-loop-forward": asOscA(sampleOsc, "sample", { kParamLoopMode: "kForward", kParamLoopStart: 30, kParamLoopEnd: 60 }),
  "sample-pingpong": asOscA(sampleOsc, "sample", { kParamLoopMode: "kPingPong", kParamLoopStart: 30, kParamLoopEnd: 60 }),
  "sample-octave-up": asOscA(sampleOsc, "sample", { kParamOctave: { $float: 1 } }),
  "sample-start-50": asOscA(sampleOsc, "sample", { kParamStart: 50 }),
  "multi-plain": asOscA(multiOsc, "multi"),
  "multi-octave-down": asOscA(multiOsc, "multi", { kParamOctave: { $float: -1 } }),
  "multi-kalimba": asOscA(kalimbaOsc, "multi"),
  // Filter responses (render at a low note so harmonics sample |H(f)| densely; `filt-off` is the reference).
  "filt-off": [],
  ...Object.fromEntries(FILTER_TYPES.map((t) => [`filt-${t}`, filter(t, 0.5)])),
  ...Object.fromEntries(FILTER_TOP.flatMap((t) => [0, 50, 90].map((r) => [`filt-${t}-reso-${r}`, filter(t, 0.5, r)]))),
  ...Object.fromEntries(["MgL12", "L12", "H12", "B12", "MgL24", "H24"].flatMap((t) => [0.25, 0.75].map((c) => [`filt-${t}-freq-${c}`, filter(t, c)]))),
  ...Object.fromEntries([["MgL12", 0.5], ["L12", 0.5], ["H12", 0.5], ["LadderEMS", 0.5]].flatMap(([t, c]) => [25, 50, 80, 100].map((d) => [`filt-${t}-drive-${d}`, [...filter(t, c), ["VoiceFilter0", "kParamDrive", d]]]))),
  ...Object.fromEntries(FILTER_VAR.flatMap((t) => [50, 100].map((v) => [`filt-${t}-var-${v}`, [...filter(t, 0.5), ["VoiceFilter0", "kParamVar", v]]]))),
  ...Object.fromEntries([0, 100].map((v) => [`filt-MgL12-stereo-${v}`, [...filter("MgL12", 0.5), ["VoiceFilter0", "kParamStereo", v]]])),
  "filt-MgL12-level-1": [...filter("MgL12", 0.5), ["VoiceFilter0", "kParamLevelOut", 1]],
  "filt-MgL12-wet-50": [...filter("MgL12", 0.3), ["VoiceFilter0", "kParamWet", 50]],
  // Finer sweeps: resonance curves, ladder cutoffs and drive, intermediate Var.
  ...Object.fromEntries(FILTER_SWEEP.flatMap((t) => [0, 10, 20, 30, 40, 50, 60, 70, 80, 90, 95, 100].map((r) => [`filt2-${t}-reso-${r}`, filter(t, 0.5, r)]))),
  ...Object.fromEntries(FILTER_LADDERS.flatMap((t) => [0.25, 0.75].map((c) => [`filt2-${t}-freq-${c}`, filter(t, c)]))),
  ...Object.fromEntries(FILTER_LADDERS.flatMap((t) => [25, 50, 100].map((d) => [`filt2-${t}-drive-${d}`, [...filter(t, 0.5), ["VoiceFilter0", "kParamDrive", d]]]))),
  ...Object.fromEntries(FILTER_VAR.flatMap((t) => [25, 75].map((v) => [`filt2-${t}-var-${v}`, [...filter(t, 0.5), ["VoiceFilter0", "kParamVar", v]]]))),
  ...Object.fromEntries([0, 1, 2].flatMap((o) => [[`filt2-os${o}-L12-freq-0.75`, [...filter("L12", 0.75), ["Global0", "kParamOversampling", o]]], [`filt2-os${o}-L12-reso-0`, [...filter("L12", 0.5, 0), ["Global0", "kParamOversampling", o]]]])),
  // Delay-based families (comb, flange, phase), stereo spread and the drive-0 clipper.
  ...Object.fromEntries(FILTER_DELAY.flatMap((t) => [0, 10, 20, 30, 40, 50, 60, 70, 80, 90, 100].map((r) => [`filt3-${t}-reso-${r}`, filter(t, 0.5, r)]))),
  ...Object.fromEntries(["CombP", "CombN", "FlangeP", "FlangeN", "Phase24P", "Phase48P", "CombL6P", "CombHL6P"].flatMap((t) => [0.3, 0.45, 0.7].map((c) => [`filt3-${t}-freq-${c}`, filter(t, c, 50)]))),
  ...Object.fromEntries(["CombP", "CombL6P", "FlangeP", "Phase24P", "Comb2", "Combs", "Allpasses"].flatMap((t) => [0, 50, 100].map((v) => [`filt3-${t}-var-${v}-reso-70`, [...filter(t, 0.5, 70), ["VoiceFilter0", "kParamVar", v]]]))),
  "filt3-MgL12-stereo-25": [...filter("MgL12", 0.5), ["VoiceFilter0", "kParamStereo", 25]],
  "filt3-loud-off": LOUD,
  "filt3-loud-L12-open": [...LOUD, ...filter("L12", 1, 0)],
  "filt3-loud-L12-open-drive-10": [...LOUD, ...filter("L12", 1, 0), ["VoiceFilter0", "kParamDrive", 10]],
  // Var against cutoff: is a Var-set frequency absolute or relative to the cutoff?
  ...Object.fromEntries(["LH12", "CombL6P", "LBH24"].flatMap((t) => [0.3, 0.7].flatMap((c) => [25, 50, 75].map((v) => [`filt4-${t}-freq-${c}-var-${v}`, [...filter(t, c, t === "CombL6P" ? 70 : 10), ["VoiceFilter0", "kParamVar", v]]])))),
  "fx-split-dist-high": rack(fx("FXSplit", { kParamFreq: 1000, kParamModuleCount1: 0, kParamModuleCount2: 1 }), fx("FXDistortion", { kParamDrive: 80 })),
};

mkdirSync(outDir, { recursive: true });
for (const old of readdirSync(outDir)) if (old.endsWith(".SerumPreset")) unlinkSync(join(outDir, old));
const metadata = { fileType: "SerumPreset", presetAuthor: "lumen-probes", presetDescription: "", product: "Serum2", productVersion: init.metadata.productVersion, tags: [], url: "", vendor: "Xfer Records", version: init.metadata.version };
let i = 0;
for (const [name, changes] of Object.entries(probes)) {
  if (selected && name !== "init" && !selected.test(name)) continue;
  const payload = probe(changes);
  payload.presetName = name;
  writeFileSync(join(outDir, `${String(i++).padStart(2, "0")} ${name}.SerumPreset`), encodePreset({ metadata: { ...metadata, presetName: name }, version: 2, payload }));
}
console.log(`wrote ${i} probes to ${outDir}`);
