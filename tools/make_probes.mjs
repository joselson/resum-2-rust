// Generates calibration probe presets: Serum 2's Init state with one change each.
// usage: node tools/make_probes.mjs INIT_PROCESSOR_STATE OUT_DIR [NAME_REGEX]
// LUMEN_FACTORY_PRESETS=DIR adds `fxrvf-` probes: reverb modules read from the factory
// presets at run time (no factory settings are stored here).
//
// INIT_PROCESSOR_STATE is the decoded IComponent block of a fresh Serum 2
// instance (an XferJson container). Each probe isolates one behaviour so a
// render comparison points at a single mapping.
import { readFileSync, writeFileSync, mkdirSync, readdirSync, unlinkSync } from "node:fs";
import { join } from "node:path";
import { execFileSync } from "node:child_process";

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
    if (change[0] === "__mod") {
      // [__mod, slot, [source, aux], destModuleTypeString, destModuleID, destParamName, destParamID, {slot params}]
      const [, slot, source, type, id, name, paramId, params, extra = {}] = change;
      payload[`ModSlot${slot}`] = { destModuleID: id, destModuleParamID: paramId, destModuleParamName: name, destModuleTypeString: type, plainParams: Object.fromEntries(Object.entries(params).map(([k, v]) => [k, typeof v === "string" ? v : f(v)])), source, ...extra };
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
const RP0 = [["Oscillator0.WTOsc0", "kParamRandomPhase", 0]];
const uni = (n, detune) => [["Oscillator0", "kParamUnison", n], ["Oscillator0", "kParamDetune", detune]];
const curveData = (pts, tensions) => ({ curveVals: [...tensions, 0.5], numPoints: pts.length - 1, xVals: pts.map(([x]) => f(x)), yVals: pts.map(([, y]) => f(1 - y)) });
const slotVol = (source, params, extra) => ["__mod", 0, source, "Oscillator", 0, "kParamVolume", 1, params, extra];
const SLOW_LFO = [["Oscillator0", "kParamVolume", 0], ["Oscillator0.WTOsc0", "kParamRandomPhase", 0], ["LFO0", "kParamBeatSync", 0], ["LFO0", "kParamRate", 0.5]];
// Env 2: instant attack, full sustain, instant release; the amp envelope releases slowly so the fall is visible.
const ENV2_STEP = [["Oscillator0", "kParamVolume", 0], ["Oscillator0.WTOsc0", "kParamRandomPhase", 0], ["Env1", "kParamAttack", 0], ["Env1", "kParamSustain", 1], ["Env1", "kParamRelease", 0], ["Env0", "kParamRelease", 2]];
// Env 2 drives OSC A's volume from 0; the amp envelope releases slowly so Env 2's release shows.
const ENV2_VOL = [["Oscillator0", "kParamVolume", 0], ["Oscillator0.WTOsc0", "kParamRandomPhase", 0], ["Env0", "kParamRelease", 3], slotVol([3, 0], { kParamAmount: 100 })];
const SUB_SINE = [["Oscillator4", "kParamEnable", 1], ["Oscillator4", "kParamVolume", 1], ["Oscillator0", "kParamEnable", 0]];
const DIST_MODES = ["kTube", "kSoftClip", "kHardClip", "kDiode1", "kDiode2", "kLinFold", "kSinFold", "kZeroSquare", "kDownsample", "kAsym", "kRectify", "kXShaper", "kXShaperAsym", "kSineShaper", "kStompBox", "kTapeSat", "kOverdrive", "kSoftSat"];
const DIST_SHAPED = ["kTube", "kSoftClip", "kHardClip", "kDiode1", "kDiode2", "kAsym", "kStompBox", "kTapeSat", "kOverdrive", "kSoftSat", "kZeroSquare"];
// A click: OSC A at C5 with an instant amp envelope (decay and release at minimum, sustain 0).
const CLICK = [["Oscillator0.WTOsc0", "kParamRandomPhase", 0], ["Env0", "kParamAttack", 0], ["Env0", "kParamDecay", 0.002], ["Env0", "kParamSustain", 0], ["Env0", "kParamRelease", 0.002]];
const LFO_VOL = [["Oscillator0", "kParamVolume", 0], ["Oscillator0.WTOsc0", "kParamRandomPhase", 0], ["__mod", 0, [6, 0], "Oscillator", 0, "kParamVolume", 1, { kParamAmount: 100 }]];
// Compressor probe input levels (FX Utility output per stage) and settings.
const COMP_LEVELS = [0.05, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1].map((l) => [`u${l}`, [l]]).concat([["u1x2", [1, 1]]]);
const COMP_SETTINGS = {
  default: {},
  ...Object.fromEntries([0, 0.1, 0.25, 0.5, 0.75, 1].map((t) => [`limit-${t}`, { kParamThresh: t, kParamRatio: 1000000 }])),
  ...Object.fromEntries([1, 1.5, 2, 4, 10].map((r) => [`ratio-${r}`, { kParamThresh: 0.5, kParamRatio: r }])),
  ...Object.fromEntries([6, 31].map((m) => [`makeup-${m}`, { kParamThresh: 0.5, kParamRatio: 4, kParamMakeup: m }])),
  ...Object.fromEntries([0.25, 0.75].map((o) => [`out-${o}`, { kParamThresh: 0.5, kParamRatio: 4, kParamLevelOut: o }])),
  "wet-50": { kParamThresh: 0.5, kParamRatio: 1000000, kParamWet: 50 },
};
const MB_SETTINGS = {
  default: {},
  xover: { kParamXoverLow: 300, kParamXoverHi: 2500 },
  ...Object.fromEntries([0, 1, 2].flatMap((b) => [[`thud${b}-50`, { [`kParamThreshUD${b}`]: 50 }], [`thud${b}-150`, { [`kParamThreshUD${b}`]: 150 }], [`gain${b}-12`, { [`kParamGain${b}`]: 12 }]])),
  "ratio0-0": { kParamRatio0: 0 }, "below0-0": { kParamRatioBelow0: 0 }, "below-1": { kParamRatioBelow: 1 },
  "deadband0-24": { kParamDeadband0: 24 }, "deadband0-3": { kParamDeadband0: 3 },
  "thresh-0.75": { kParamThresh: 0.75 }, "ratio-1e6": { kParamRatio: 1000000 },
  ...Object.fromEntries([0.25, 0.3, 0.4, 0.45, 0.55, 0.6, 0.9].map((t) => [`thresh-${t}`, { kParamThresh: t }])),
  ...Object.fromEntries([0, 1, 2].flatMap((b) => [3, 24].map((d) => [`below-1-db${b}-${d}`, { kParamRatioBelow: 1, [`kParamDeadband${b}`]: d }]))),
  "below-0.5": { kParamRatioBelow: 0.5 }, ...Object.fromEntries([0.25, 0.5, 0.75, 0.9].map((v) => [`below-1-below1-${v}`, { kParamRatioBelow: 1, kParamRatioBelow1: v }])),
  "ratio1-0.5": { kParamRatio1: 0.5 }, "ratio-2": { kParamRatio: 2 }, "ratio-10": { kParamRatio: 10 },
  fast: { kParamAttack: 0.1, kParamRelease: 30 }, "att-10": { kParamAttack: 10 }, "rel-10": { kParamRelease: 10 },
  "below-1-fast": { kParamRatioBelow: 1, kParamAttack: 0.1, kParamRelease: 30 }, "wet-50": { kParamWet: 50 }, "makeup-4": { kParamMakeup: 4 },
};
/** Short reverb control names in probe ids (tools/fit_reverb.py reads them back). */
const REVERB_CONTROL = { kParamSize: "size", kParamDelay: "delay", kParamFeedback: "fb", kParamFreqB: "freqb", kParamFreq: "freq", kParamWidth: "width",
  kParamFreqC: "freqc", kParamMode: "mode", kParamVintageScale: "vscale", kParamVintageScaleB: "vscaleb", kParamPreDelay: "predelay" };
/** Every combination of the given control values, as [type, params] pairs, plus each
 * value alone: tools/fit_reverb.py takes a pair's interaction against the single sweeps,
 * which must hold every grid value (or each pair re-corrects their interpolation). */
function grid(type, axes) {
  const singles = Object.entries(axes).flatMap(([k, vs]) => vs.map((v) => [type, { [k]: v }]));
  return [...singles, ...Object.entries(axes).reduce((acc, [k, vs]) => acc.flatMap((q) => vs.map((v) => ({ ...q, [k]: v }))), [{}]).map((q) => [type, q])];
}
/** The three reverb modules per type (by stored type) that set the most controls. */
function factoryReverbs(dir) {
  if (!dir) return {};
  const files = execFileSync("find", [dir, "-iname", "*.SerumPreset"], { encoding: "utf8" }).trim().split("\n").sort();
  const byType = {};
  for (const file of files) {
    const payload = decodePreset(readFileSync(file)).payload;
    for (const rack of ["FXRack0", "FXRack1", "FXRack2"]) for (const m of payload[rack]?.FX ?? []) {
      const pp = m.FXReverb?.plainParams;
      if (!pp || typeof pp !== "object" || pp.kParamPreDelayBeatSync) continue;
      (byType[pp.kParamType ?? "unnamed"] ??= []).push(pp);
    }
  }
  const out = {};
  for (const [type, list] of Object.entries(byType)) {
    list.sort((a, b) => Object.keys(b).length - Object.keys(a).length).slice(0, 3).forEach((pp, i) => {
      const params = Object.fromEntries(Object.entries(pp).filter(([k]) => !["kParamWet", "kParamLevelOut", "kParamEnable"].includes(k)).map(([k, v]) => [k, typeof v === "object" ? v.$float : v]));
      out[`fxrvf-${type.replace(/^k/, "").toLowerCase()}-${i}`] = [...CLICK, ["Env0", "kParamSustain", 1e-3], ...rack(fx("FXReverb", { ...params, kParamWet: 100 }))];
    });
  }
  return out;
}

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
  // Oscillator core (osc2-): high-note band-limiting, unison law, sub shapes, envelope curves.
  // Random phase 0 so unison voices start coherent: detune 0 measures the level law
  // and resolved fundamentals give each voice's detune and pan.
  ...Object.fromEntries([1, 2, 3, 4].flatMap((o) => [0, 4, 8].map((st) => [`osc2-pitch-oct-${o}-semi-${st}`, [...RP0, ["Oscillator0", "kParamOctave", o], ["Oscillator0", "kParamPitch", st]]]))),
  ...Object.fromEntries([2, 3, 4, 5, 7, 8, 16].flatMap((n) => [[`osc2-uni-${n}-coherent`, [...RP0, ...uni(n, 0)]], [`osc2-uni-${n}`, [...RP0, ...uni(n, 0.25)]]])),
  ...Object.fromEntries([0.05, 0.1, 0.5, 1].map((d) => [`osc2-uni-7-detune-${d}`, [...RP0, ...uni(7, d)]])),
  ...Object.fromEntries(["kDetuneSuper", "kDetuneExp", "kDetuneInv", "kDetuneRandom"].map((m) => [`osc2-uni-7-${m.slice(7).toLowerCase()}`, [...RP0, ...uni(7, 0.5), ["Oscillator0", "kParamDetuneMode", m]]])),
  ...Object.fromEntries([0, 25, 50, -100].map((w) => [`osc2-uni-7-width-${w}`, [...RP0, ...uni(7, 0.5), ["Oscillator0", "kParamUnisonStereo", w]]])),
  ...Object.fromEntries([0, 25, 50, 100].flatMap((b) => [[`osc2-uni-7-blend-${b}-coherent`, [...RP0, ...uni(7, 0), ["Oscillator0", "kParamDetuneWid", b]]], [`osc2-uni-4-blend-${b}-coherent`, [...RP0, ...uni(4, 0), ["Oscillator0", "kParamDetuneWid", b]]]])),
  ...Object.fromEntries([12, 48].map((r) => [`osc2-uni-7-range-${r}`, [...RP0, ...uni(7, 0.5), ["Oscillator0", "kParamUnisonRange", r]]])),
  ...Object.fromEntries(["kSine", "kRoundRect", "kTriangle", "kSaw", "kSquare", "kPulse"].flatMap((sh) => [-1, 2].map((o) => [`osc2-sub-${sh.slice(1).toLowerCase()}-oct-${o}`, [["Oscillator4", "kParamEnable", 1], ["Oscillator4.SubOsc4", "kParamShape", sh], ["Oscillator4", "kParamOctave", o], ["Oscillator0", "kParamEnable", 0]]]))),
  ...Object.fromEntries([0, 10, 25, 40, 60, 75, 90, 100].flatMap((c) => [
    [`osc2-env-attack-curve-${c}`, [...RP0, ["Env0", "kParamAttack", 0.5], ["Env0", "kParamCurve1", c]]],
    [`osc2-env-decay-curve-${c}`, [...RP0, ["Env0", "kParamDecay", 0.5], ["Env0", "kParamSustain", 0], ["Env0", "kParamCurve2", c]]],
    [`osc2-env-release-curve-${c}`, [...RP0, ["Env0", "kParamRelease", 0.45], ["Env0", "kParamCurve3", c]]],
  ])),
  // Second unison pass (osc3-): detuned blend sweeps, super spacing per count, width, stacks.
  ...Object.fromEntries([3, 4, 7].flatMap((n) => [0, 10, 25, 40, 50, 60, 90, 100].map((b) => [`osc3-uni-${n}-blend-${b}`, [...RP0, ...uni(n, 0.25), ["Oscillator0", "kParamDetuneWid", b]]]))),
  ...Object.fromEntries([3, 4, 5, 8, 16].flatMap((n) => ["kDetuneSuper", "kDetuneExp", "kDetuneInv"].map((m) => [`osc3-uni-${n}-${m.slice(7).toLowerCase()}`, [...RP0, ...uni(n, 0.5), ["Oscillator0", "kParamDetuneMode", m]]]))),
  ...Object.fromEntries([10, 75, -50].map((w) => [`osc3-uni-7-width-${w}`, [...RP0, ...uni(7, 0.5), ["Oscillator0", "kParamUnisonStereo", w]]])),
  "osc3-uni-2-width-0": [...RP0, ...uni(2, 0.5), ["Oscillator0", "kParamUnisonStereo", 0]],
  ...Object.fromEntries(["kOctave1", "kOctave2", "kOctaveFifth1", "kCenter12", "kCenter24"].map((st) => [`osc3-uni-4-stack-${st.slice(1).toLowerCase()}`, [...RP0, ...uni(4, 0.25), ["Oscillator0", "kParamUnisonStack", st]]])),
  "osc3-single": RP0,
  // Third pass (osc4-): envelope curve constants, amp sustain law, unison stack voice assignment.
  ...Object.fromEntries([2, 5, 15, 20, 30, 35, 45, 50, 55, 65, 70, 80, 85, 95, 98].map((c) => [`osc4-env-attack-curve-${c}`, [...RP0, ["Env0", "kParamAttack", 0.5], ["Env0", "kParamCurve1", c]]])),
  ...Object.fromEntries([[0.5, 25], [0.5, 66.6], [0.2, 90]].map(([sus, c]) => [`osc4-env-decay-sus-${sus}-curve-${c}`, [...RP0, ["Env0", "kParamDecay", 0.5], ["Env0", "kParamSustain", sus], ["Env0", "kParamCurve2", c]]])),
  "osc4-env-release-sus-0.5": [...RP0, ["Env0", "kParamSustain", 0.5], ["Env0", "kParamRelease", 0.45], ["Env0", "kParamCurve3", 25]],
  ...Object.fromEntries([3, 5, 7, 8].flatMap((n) => ["kOctave1", "kOctave2", "kOctave3", "kOctaveFifth1", "kOctaveFifth2", "kOctaveFifth3", "kCenter12", "kCenter24"].map((st) => [`osc4-uni-${n}-stack-${st.slice(1).toLowerCase()}`, [...RP0, ...uni(n, 0.25), ["Oscillator0", "kParamUnisonStack", st]]]))),
  "osc4-single": RP0,
  // LFO timing (lfo-): LFO 1 drives OSC A's volume from 0, so the output level traces the LFO.
  ...Object.fromEntries([0.3, 0.35, 0.4, 0.425, 0.45, 0.475, 0.5, 0.525, 0.55, 0.575, 0.6, 0.625, 0.65, 0.675, 0.7, 0.725, 0.75, 0.775, 0.8].map((x) => [`lfo-sync-x${x}`, [...LFO_VOL, ["LFO0", "kParamRate", 100 * x ** 4]]])),
  ...Object.fromEntries([0.25, 0.5, 1, 2, 4, 8, 16, 32].map((r) => [`lfo-free-${r}`, [...LFO_VOL, ["LFO0", "kParamBeatSync", 0], ["LFO0", "kParamRate", r]]])),
  ...Object.fromEntries([["dotted", "kParamDotted"], ["triplet", "kParamTriplets"], ["x10", "kParamRate10x"]].flatMap(([n, k]) => [6.25, 12.96].map((r) => [`lfo-sync-${n}-${r}`, [...LFO_VOL, ["LFO0", "kParamRate", r], ["LFO0", k, 1]]]))),
  ...Object.fromEntries([["dotted", "kParamDotted"], ["triplet", "kParamTriplets"], ["x10", "kParamRate10x"]].map(([n, k]) => [`lfo-free-2-${n}`, [...LFO_VOL, ["LFO0", "kParamBeatSync", 0], ["LFO0", "kParamRate", 2], ["LFO0", k, 1]]])),
  ...Object.fromEntries(["Lorenz", "Rossler", "RandomSH"].flatMap((t) => [[`lfo-${t.toLowerCase()}-sync`, [...LFO_VOL, ["LFO0", "kParamType", t]]], [`lfo-${t.toLowerCase()}-free-1`, [...LFO_VOL, ["LFO0", "kParamType", t], ["LFO0", "kParamBeatSync", 0], ["LFO0", "kParamRate", 1]]], [`lfo-${t.toLowerCase()}-free-4`, [...LFO_VOL, ["LFO0", "kParamType", t], ["LFO0", "kParamBeatSync", 0], ["LFO0", "kParamRate", 4]]]])),
  "lfo-bipolar": [...LFO_VOL.slice(0, -1), ["__mod", 0, [6, 0], "Oscillator", 0, "kParamVolume", 1, { kParamAmount: 50, kParamBipolar: 1 }], ["Oscillator0", "kParamVolume", 0.5]],
  "lfo-off": [["Oscillator0", "kParamVolume", 0], ["Oscillator0.WTOsc0", "kParamRandomPhase", 0]],
  // Mod slot options (mod-): LFO 1 free at 0.5 Hz sweeps the curves; Env 2 steps for smoothing; Fixed for static depth.
  ...Object.fromEntries([-100, -50, -20, 20, 50, 100].map((c) => [`mod-curvein-${c}`, [...SLOW_LFO, slotVol([6, 0], { kParamAmount: 100, kParamCurveIn: c })]])),
  ...Object.fromEntries([["t02", curveData([[0, 0], [1, 1]], [0.2])], ["t08", curveData([[0, 0], [1, 1]], [0.8])], ["peak", curveData([[0, 0], [0.5, 1], [1, 0.25]], [0.5, 0.5])]].map(([n, c]) => [`mod-main-${n}`, [...SLOW_LFO, slotVol([6, 0], { kParamAmount: 100, kParamMainCurveData: 1 }, { flex: [c, {}] })]])),
  "mod-main-fall-t02": [...SLOW_LFO, slotVol([6, 0], { kParamAmount: 100, kParamMainCurveData: 1 }, { flex: [curveData([[0, 1], [1, 0]], [0.2]), {}] })],
  ...Object.fromEntries([["rise", [[0, 0], [0.5, 1], [1, 0]], [0.2, 0.5]], ["fall", [[0, 0], [0.5, 1], [1, 0]], [0.5, 0.2]]].map(([n, pts, t]) => [`mod-lfo-shape-${n}-t02`, [...SLOW_LFO, ["__section", "LFO0", { curveData: curveData(pts, t), pathData: {}, plainParams: { kParamBeatSync: f(0), kParamRate: 0.5 } }], slotVol([6, 0], { kParamAmount: 100 })]])),
  ...Object.fromEntries([-50, 50, 100].map((c) => [`mod-curveout-${c}`, [...SLOW_LFO, slotVol([6, 0], { kParamAmount: 100, kParamCurveOut: c })]])),
  "mod-curveout-50-bipolar": [...SLOW_LFO, ["Oscillator0", "kParamVolume", 0.25], slotVol([6, 0], { kParamAmount: 100, kParamBipolar: 1, kParamCurveOut: 50 })],
  "mod-auxcurve-50": [...SLOW_LFO, slotVol([38, 6], { kParamAmount: 100, kParamAuxCurve: 50 })],
  "mod-aux-t02": [...SLOW_LFO, slotVol([38, 6], { kParamAmount: 100, kParamAuxCurveData: 1 }, { flex: [{}, curveData([[0, 0], [1, 1]], [0.2])] })],
  "mod-aux-plain": [...SLOW_LFO, slotVol([38, 6], { kParamAmount: 100 })],
  "mod-out-50": [...SLOW_LFO, slotVol([6, 0], { kParamAmount: 100, kParamOut: 50 })],
  "mod-plain": [...SLOW_LFO, slotVol([6, 0], { kParamAmount: 100 })],
  ...Object.fromEntries([[25, 25, 1], [50, 50, 1], [100, 100, 1], [25, 100, 0], [100, 25, 0]].map(([r, fl, link]) => [`mod-smooth-r${r}-f${fl}-l${link}`, [...ENV2_STEP, slotVol([3, 0], { kParamAmount: 100, kParamSmoothRise: r, kParamSmoothFall: fl, kParamSmoothLink: link })]])),
  "mod-smooth-none": [...ENV2_STEP, slotVol([3, 0], { kParamAmount: 100 })],
  ...Object.fromEntries([[10, 0], [50, 0], [100, 0], [100, 1], [20, 1]].map(([a, b]) => [`mod-tuning-${a}${b ? "-bi" : ""}`, [["__mod", 0, [38, 0], "Global", 0, "kParamMasterTuning", 1, { kParamAmount: a, kParamBipolar: b }]]])),
  ...Object.fromEntries([1, 2, -5].map((a) => [`mod-tuning-fine-${a}`, [["__mod", 0, [38, 0], "Global", 0, "kParamMasterTuning", 1, { kParamAmount: a }]]])),
  "mod-tuning-static-0.55": [["Global0", "kParamMasterTuning", 0.55]],
  ...Object.fromEntries(["Lorenz", "Rossler", "RandomSH", "Path"].flatMap((t) => [[`mod-lfoy-${t.toLowerCase()}`, [...SLOW_LFO, ["LFO0", "kParamRate", 1], ["LFO0", "kParamType", t], slotVol([39, 0], { kParamAmount: 100 })]], [`mod-lfox-${t.toLowerCase()}`, [...SLOW_LFO, ["LFO0", "kParamRate", 1], ["LFO0", "kParamType", t], slotVol([6, 0], { kParamAmount: 100 })]]])),
  "mod-lfoy-default": [...SLOW_LFO, slotVol([39, 0], { kParamAmount: 100 })],
  ...Object.fromEntries([["tri", [[0.1, 0.5], [0.9, 0.5], [0.5, 0.6]], [0.5, 0.5, 0.5], false], ["tri-t02", [[0.1, 0.5], [0.9, 0.5], [0.5, 0.6]], [0.2, 0.5, 0.5], false], ["open", [[0.1, 0.2], [0.9, 0.5], [0.3, 0.9]], [0.5, 0.5, 0.5], true]].flatMap(([n, pts, t, open]) => [[39, "y"], [6, "x"]].map(([src, axis]) => [`mod-path-${n}-${axis}`, [...SLOW_LFO, ["__section", "LFO0", { curveData: {}, pathData: { curveVals: t, isOpen: open, numPoints: pts.length, xVals: pts.map(([x]) => x), yVals: pts.map(([, y]) => y) }, pathDisplayName: "Custom", plainParams: { kParamBeatSync: f(0), kParamRate: 0.5, kParamType: "Path" } }], slotVol([src, 0], { kParamAmount: 100 })]]))),
  // Global voice amp; envelope start/end and beat sync; LFO direction, anchoring and mono.
  ...Object.fromEntries([0, 0.25, 0.75, 1].map((a) => [`mod-voiceamp-${a}`, [["Global0", "kParamVoiceAmp", a]]])),
  "mod-voiceamp-mod-25": [["__mod", 0, [38, 0], "Global", 0, "kParamVoiceAmp", 2, { kParamAmount: 25 }]],
  "mod-env-start-0.3": [...ENV2_VOL, ["Env1", "kParamAttack", 0.5], ["Env1", "kParamStart", 0.3]],
  "mod-env-end-0.4": [...ENV2_VOL, ["Env1", "kParamRelease", 0.5], ["Env1", "kParamEnd", 0.4]],
  "mod-env-start-end-plain": [...ENV2_VOL, ["Env1", "kParamAttack", 0.5], ["Env1", "kParamRelease", 0.5]],
  ...Object.fromEntries([0.25, 1, 2].map((a) => [`mod-env-sync-attack-${a}`, [...ENV2_VOL, ["Env1", "kParamBeatSync", 1], ["Env1", "kParamAttack", a]]])),
  ...Object.fromEntries([["dir-0", [["LFO0", "kParamDirection", 0]]], ["dir-1", [["LFO0", "kParamDirection", 1]]], ["dir-2", [["LFO0", "kParamDirection", 2]]], ["anchored-0", [["LFO0", "kParamAnchored", 0]]], ["mono", [["LFO0", "kParamMono", 1]]], ["anchored-0-phase-90", [["LFO0", "kParamAnchored", 0], ["LFO0", "kParamPhase", 90]]], ["phase-90", [["LFO0", "kParamPhase", 90]]]].map(([n, extra]) => [`mod-lfo-${n}`, [...SLOW_LFO, ["__section", "LFO0", { curveData: curveData([[0, 0], [0.98, 1], [1, 0]], [0.5, 0.5]), pathData: {}, plainParams: { kParamBeatSync: f(0), kParamRate: 0.5 } }], ...extra, slotVol([6, 0], { kParamAmount: 100 })]])),
  "mod-tuning-out-50": [["__mod", 0, [38, 0], "Global", 0, "kParamMasterTuning", 1, { kParamAmount: 100, kParamOut: 50 }]],
  // FX frequency responses (fxr-): C1 through one module; `fxr-off` is the dry reference.
  "fxr-off": [],
  ...Object.fromEntries([
    ...[0, 30, 60, 100].flatMap((q) => [[`eq-b1-t0-200-q${q}`, { kParamType1: 0, kParamFreq1: 200, kParamGain1: 12, kParamReso1: q }], [`eq-b1-t1-200-q${q}`, { kParamType1: 1, kParamFreq1: 200, kParamGain1: 12, kParamReso1: q }], [`eq-b1-t2-200-q${q}`, { kParamType1: 2, kParamFreq1: 200, kParamReso1: q }], [`eq-b2-t0-4000-q${q}`, { kParamType2: 0, kParamFreq2: 4000, kParamGain2: 12, kParamReso2: q }], [`eq-b2-t1-4000-q${q}`, { kParamType2: 1, kParamFreq2: 4000, kParamGain2: 12, kParamReso2: q }], [`eq-b2-t2-4000-q${q}`, { kParamType2: 2, kParamFreq2: 4000, kParamReso2: q }]]),
    ...[50, 1000, 4000].map((fr) => [`eq-b1-t1-${fr}`, { kParamType1: 1, kParamFreq1: fr, kParamGain1: 12 }]),
    ...[100, 1000].map((fr) => [`eq-b2-t1-${fr}`, { kParamType2: 1, kParamFreq2: fr, kParamGain2: 12 }]),
    ...[-12, 6, 24].map((g) => [`eq-b1-t1-gain${g}`, { kParamType1: 1, kParamGain1: g }]),
    ...[-12, 24].map((g) => [`eq-b1-t0-gain${g}`, { kParamType1: 0, kParamGain1: g }]),
    ["eq-b1-t2-1000", { kParamType1: 2, kParamFreq1: 1000 }], ["eq-b2-t2-1000", { kParamType2: 2, kParamFreq2: 1000 }],
    ["eq-b1-t0-1000", { kParamType1: 0, kParamFreq1: 1000, kParamGain1: 12 }], ["eq-b2-t0-1000", { kParamType2: 0, kParamFreq2: 1000, kParamGain2: 12 }],
    ["eq-defaults", {}],
  ].map(([n, params]) => [`fxr-${n}`, rack(fx("FXEQ", params))])),
  // Distortion pre/post filter alone: Downsample at drive 0 passes the signal unchanged.
  ...Object.fromEntries([
    ...[0, 25, 50, 75, 100].map((m) => [`dflt-lphp${m}`, { kParamLPHP: m }]),
    ...[0.3, 0.7].flatMap((f) => [0, 50, 100].map((m) => [`dflt-lphp${m}-f${f}`, { kParamLPHP: m, kParamFreq: f }])),
    ...[0.5, 4, 7].flatMap((bw) => [0, 50].map((m) => [`dflt-lphp${m}-bw${bw}`, { kParamLPHP: m, kParamBW: bw }])),
    ["dflt-post-lphp50", { kParamLPHP: 50, kParamPrePost: 2 }],
  ].map(([n, params]) => [`fxr-${n}`, rack(fx("FXDistortion", { kParamMode: "kDownsample", kParamDrive: 0, kParamPrePost: 1, ...params }))])),
  // Distortion transfer curves (fxd-): the sub's sine at C1 through each mode; `fxd-off` is dry.
  "fxd-off": SUB_SINE,
  ...Object.fromEntries(DIST_MODES.flatMap((m) => [0, 25, 50, 100].map((d) => [`fxd-${m.slice(1).toLowerCase()}-${d}`, [...SUB_SINE, ...rack(fx("FXDistortion", { kParamMode: m, kParamDrive: d }))]]))),
  // Distortion drive law and input-level check (fxd2-).
  "fxd2-off": SUB_SINE,
  "fxd2-off-quiet": [...SUB_SINE, ["Oscillator4", "kParamVolume", 0.25]],
  ...Object.fromEntries(DIST_SHAPED.flatMap((m) => [10, 40, 60, 75, 90].map((d) => [`fxd2-${m.slice(1).toLowerCase()}-${d}`, [...SUB_SINE, ...rack(fx("FXDistortion", { kParamMode: m, kParamDrive: d }))]]))),
  ...Object.fromEntries(DIST_SHAPED.map((m) => [`fxd2-${m.slice(1).toLowerCase()}-50-quiet`, [...SUB_SINE, ["Oscillator4", "kParamVolume", 0.25], ...rack(fx("FXDistortion", { kParamMode: m, kParamDrive: 50 }))]])),
  // Distortion over a wide input range (fxd3-): one or two FX Utility stages at +12 dB first.
  "fxd3-off": SUB_SINE,
  "fxd3-boost-1": [...SUB_SINE, ...rack(fx("FXUtils", { kParamLevelOut: 1 }))],
  "fxd3-boost-2": [...SUB_SINE, ...rack(fx("FXUtils", { kParamLevelOut: 1 }), fx("FXUtils", { kParamLevelOut: 1 }))],
  ...Object.fromEntries([...DIST_SHAPED, "kDiode1"].flatMap((m) => [[1, 100], [2, 100], [2, 0], [2, 50]].map(([n, d]) => [`fxd3-${m.slice(1).toLowerCase()}-${d}-boost-${n}`, [...SUB_SINE, ...rack(...Array.from({ length: n }, () => fx("FXUtils", { kParamLevelOut: 1 })), fx("FXDistortion", { kParamMode: m, kParamDrive: d }))]]))),
  // Distortion curve tables (fxd4-): every static mode × 9 drives × 3 input ranges.
  "fxd4-off": SUB_SINE,
  "fxd4-boost-1": [...SUB_SINE, ...rack(fx("FXUtils", { kParamLevelOut: 1 }))],
  "fxd4-boost-2": [...SUB_SINE, ...rack(fx("FXUtils", { kParamLevelOut: 1 }), fx("FXUtils", { kParamLevelOut: 1 }))],
  ...Object.fromEntries([...DIST_SHAPED, "kDiode1"].flatMap((m) => [0, 10, 25, 40, 50, 60, 75, 90, 100].flatMap((d) => [0, 1, 2].map((n) => [`fxd4-${m.slice(1).toLowerCase()}-${d}-b${n}`, [...SUB_SINE, ...rack(...Array.from({ length: n }, () => fx("FXUtils", { kParamLevelOut: 1 })), fx("FXDistortion", { kParamMode: m, kParamDrive: d }))]])))),
  // Delay taps (fxdl-): a click (instant amp envelope) through the delay at 100 % wet.
  "fxdl-dry": CLICK,
  ...Object.fromEntries([
    ["default", {}],
    ...[0.0625, 0.125, 0.25, 0.375, 0.5].map((t) => [`sync-${t}`, { kParamTimeL: t, kParamTimeR: t }]),
    ...[0.05, 0.2, 0.4].map((t) => [`free-${t}`, { kParamBeatSync: 0, kParamTimeL: t, kParamTimeR: t }]),
    ...[0, 20, 60, 90].map((fb) => [`fb-${fb}`, { kParamFeedback: fb }]),
    ["mode-1", { kParamMode: 1 }], ["mode-2", { kParamMode: 2 }],
    ["offset-l-0.75", { kParamOffsetL: 0.75 }], ["offset-r-1.25", { kParamOffsetR: 1.25 }],
    ["link", { kParamLink: 1, kParamTimeR: 0.125 }], ["unlinked-r", { kParamTimeR: 0.125 }],
    ...[200, 2000, 8000].map((fr) => [`freq-${fr}`, { kParamFreq: fr }]),
    ...[0.75, 3, 8.25].map((bw) => [`bw-${bw}`, { kParamBW: bw }]),
    ["hq-0", { kParamHQ: 0 }],
  ].map(([n, params]) => [`fxdl-${n}`, [...CLICK, ...rack(fx("FXDelay", { kParamWet: 100, ...params }))]])),
  // Delay timing details (fxdl2-) and its filter response (fxr-dly-, C1 saw, no feedback).
  ...Object.fromEntries([
    ...[0.03, 0.09, 0.1, 0.2].map((t) => [`sync-${t}`, { kParamTimeL: t, kParamTimeR: t }]),
    ...[0.002, 0.01, 0.1, 0.5].map((t) => [`free-${t}`, { kParamBeatSync: 0, kParamTimeL: t, kParamTimeR: t }]),
    ["mode-2-short", { kParamMode: 2, kParamTimeL: 0.01, kParamTimeR: 0.01 }],
  ].map(([n, params]) => [`fxdl2-${n}`, [...CLICK, ...rack(fx("FXDelay", { kParamWet: 100, ...params }))]])),
  ...Object.fromEntries([
    ["default", {}], ...[100, 300, 2000, 8000].map((fr) => [`f${fr}`, { kParamFreq: fr }]),
    ...[0.75, 2, 4, 8.25].map((bw) => [`bw${bw}`, { kParamBW: bw }]), ["f300-bw2", { kParamFreq: 300, kParamBW: 2 }],
  ].map(([n, params]) => [`fxr-dly-${n}`, rack(fx("FXDelay", { kParamWet: 100, kParamFeedback: 0, kParamTimeL: 0.01, kParamTimeR: 0.01, ...params }))])),
  // Synced delay time sweep (fxdl3-): first tap time for 101 values of TimeL.
  ...Object.fromEntries(Array.from({ length: 101 }, (_, i) => 0.001 + i * 0.005).map((t) => [`fxdl3-${t.toFixed(3)}`, [...CLICK, ...rack(fx("FXDelay", { kParamWet: 100, kParamFeedback: 0, kParamTimeL: t, kParamTimeR: t }))]])),
  // Fine sweep around each synced delay threshold (fxdl4-), 0.0005 steps.
  ...Object.fromEntries([0.001, 0.006, 0.011, 0.031, 0.061, 0.106, 0.176, 0.276, 0.416].flatMap((lo) => Array.from({ length: 11 }, (_, i) => lo + i * 0.0005)).map((t) => [`fxdl4-${t.toFixed(4)}`, [...CLICK, ...rack(fx("FXDelay", { kParamWet: 100, kParamFeedback: 0, kParamTimeL: t, kParamTimeR: t }))]])),
  "fxm2-dry": [],
  "fxm2-mode2-sustain": rack(fx("FXDelay", { kParamWet: 100, kParamMode: 2 })),
  "fxm2-mode2-wet50": rack(fx("FXDelay", { kParamWet: 50, kParamMode: 2 })),
  "fxm2-mode2-free": rack(fx("FXDelay", { kParamWet: 100, kParamMode: 2, kParamBeatSync: 0, kParamTimeL: 0.2, kParamTimeR: 0.2 })),
  // Reverb impulse responses (fxrv-): a click at 100 % wet.
  "fxrv-dry": CLICK,
  ...Object.fromEntries([
    ...["kHall", "kVintage", "kAbyss", "kSpace"].flatMap((t) => [["default", {}], ["size0", { kParamSize: 0 }], ["size100", { kParamSize: 100 }], ["fb0", { kParamFeedback: 0 }], ["fb60", { kParamFeedback: 60 }], ["fb100", { kParamFeedback: 100 }], ["freqb0", { kParamFreqB: 0 }], ["freqb100", { kParamFreqB: 100 }]].map(([n, q]) => [`${t.slice(1).toLowerCase()}-${n}`, { kParamType: t, ...q }])),
    ...[["freq100", { kParamFreq: 100 }], ["freq50", { kParamFreq: 50 }], ["delay0", { kParamDelay: 0 }], ["delay250", { kParamDelay: 250 }], ["predelay0.25", { kParamPreDelay: 0.25 }], ["width0", { kParamWidth: 0 }], ["width100", { kParamWidth: 100 }], ["freqc0", { kParamFreqC: 0 }], ["freqc100", { kParamFreqC: 100 }], ["mode0", { kParamMode: 0 }], ["mode100", { kParamMode: 100 }]].map(([n, q]) => [`hall-${n}`, q]),
    ...[["vscale0", { kParamVintageScale: 0 }], ["vscale100", { kParamVintageScale: 100 }], ["vscaleb0", { kParamVintageScaleB: 0 }], ["vscaleb100", { kParamVintageScaleB: 100 }]].map(([n, q]) => [`vintage-${n}`, { kParamType: "kVintage", ...q }]),
  ].map(([n, params]) => [`fxrv-${n}`, [...CLICK, ...rack(fx("FXReverb", { kParamWet: 100, ...params }))]])),
  // Which stored reverb settings render silence in the headless rig (fxrv3-).
  ...Object.fromEntries([
    ["hall-wet50", { kParamType: "kHall", kParamWet: 50 }], ["hall-wet99", { kParamType: "kHall", kParamWet: 99 }],
    ["hall-bell", { kParamType: "kHall", kParamDelay: 28.345, kParamFreqB: 49.325, kParamSize: 58.34, kParamWet: 38.779 }],
    ["hall-bell-wet100", { kParamType: "kHall", kParamDelay: 28.345, kParamFreqB: 49.325, kParamSize: 58.34, kParamWet: 100 }],
    ["hall-size35", { kParamType: "kHall", kParamSize: 35, kParamWet: 100 }], ["hall-size10", { kParamType: "kHall", kParamSize: 10, kParamWet: 100 }],
    ["space-pianet", { kParamType: "kSpace", kParamDelay: 56.58, kParamFeedback: 4.427, kParamFreq: 66.927, kParamFreqB: 0, kParamPreDelay: 0.193, kParamSize: 9.854, kParamWet: 100, kParamWidth: 49.427 }],
    ["space-wet50", { kParamType: "kSpace", kParamWet: 50 }],
    ["vintage-altar", { kParamType: "kVintage", kParamDelay: 109.364, kParamFreq: 59.766, kParamFreqB: 36.719, kParamSize: 55.614, kParamVintageScale: 73.07, kParamWet: 100 }],
    ["vintage-wet50", { kParamType: "kVintage", kParamWet: 50 }],
  ].map(([n, params]) => [`fxrv3-${n}`, [...CLICK, ...rack(fx("FXReverb", params))]])),
  // The same on a sustained tone (Init's saw): is the whole output muted? (fxrv4-)
  ...Object.fromEntries([["hall-wet50", { kParamType: "kHall", kParamWet: 50 }], ["hall-size10-wet50", { kParamType: "kHall", kParamSize: 10, kParamWet: 50 }], ["space-wet50", { kParamType: "kSpace", kParamWet: 50 }], ["untyped-wet50", { kParamWet: 50 }]].map(([n, params]) => [`fxrv4-${n}`, rack(fx("FXReverb", params))])),
  // A click whose envelope then sustains very quietly, so the reverb input never falls silent (fxrv5-).
  ...Object.fromEntries([1e-4, 1e-3].flatMap((sus) => [["hall-size35", { kParamType: "kHall", kParamSize: 35, kParamWet: 100 }], ["space", { kParamType: "kSpace", kParamWet: 100 }], ["vintage", { kParamType: "kVintage", kParamWet: 100 }], ["dry", null]].map(([n, params]) => [`fxrv5-sus${sus}-${n}`, [...CLICK, ["Env0", "kParamSustain", sus], ...(params ? rack(fx("FXReverb", params)) : [])]]))),
  // Reverb sweeps per type on the quiet-sustain click (fxrv6-; tools/fit_reverb.py).
  "fxrv6-dry": [...CLICK, ["Env0", "kParamSustain", 1e-3]],
  ...Object.fromEntries(["kHall", "kVintage", "kAbyss", "kSpace", null].flatMap((t) => [
    ["default", {}], ...[0, 25, 50, 75, 100].map((v) => [`size${v}`, { kParamSize: v }]), ...[0, 50, 100].map((v) => [`fb${v}`, { kParamFeedback: v }]),
    ...[0, 50, 100].map((v) => [`freqb${v}`, { kParamFreqB: v }]), ...[50, 100].map((v) => [`freq${v}`, { kParamFreq: v }]), ...[0, 100, 250].map((v) => [`delay${v}`, { kParamDelay: v }]),
    ...[0.1, 0.5].map((v) => [`predelay${v}`, { kParamPreDelay: v }]), ...[0, 100].map((v) => [`width${v}`, { kParamWidth: v }]), ...[0, 100].map((v) => [`freqc${v}`, { kParamFreqC: v }]), ...[0, 100].map((v) => [`mode${v}`, { kParamMode: v }]),
    ...(t === "kVintage" ? [0, 100].flatMap((v) => [[`vscale${v}`, { kParamVintageScale: v }], [`vscaleb${v}`, { kParamVintageScaleB: v }]]) : []),
    // Finer steps for the controls that set the decay.
    ...[10, 15, 20, 40, 45, 60, 65, 85, 90].map((v) => [`size${v}`, { kParamSize: v }]), ...[10, 20, 45, 60, 80, 150, 200].map((v) => [`delay${v}`, { kParamDelay: v }]),
    ...[25, 75].flatMap((v) => [[`fb${v}`, { kParamFeedback: v }], [`freqb${v}`, { kParamFreqB: v }], [`freq${v}`, { kParamFreq: v }], [`width${v}`, { kParamWidth: v }], [`freqc${v}`, { kParamFreqC: v }], [`mode${v}`, { kParamMode: v }]]),
    ...(t === "kVintage" ? [25, 50, 85].flatMap((v) => [[`vscale${v}`, { kParamVintageScale: v }]]) : []),
  ].map(([n, q]) => [`fxrv6-${t ? t.slice(1).toLowerCase() : "untyped"}-${n}`, [...CLICK, ["Env0", "kParamSustain", 1e-3], ...rack(fx("FXReverb", { kParamWet: 100, ...(t ? { kParamType: t } : {}), ...q }))]]))),
  // Pairs of Abyss and Vintage controls on a grid (fxrv8-): their effects do not add.
  ...Object.fromEntries([
    ...[55, 60, 65, 67, 69, 70, 71, 73, 75, 80, 95].map((v) => ["vintage", { kParamVintageScale: v }]),
    ...[0.001, 0.01].map((p) => ["vintage", { kParamVintageScale: 70, kParamPreDelay: p }]),
    ...grid("vintage", { kParamVintageScale: [0, 50, 70, 85, 100], kParamPreDelay: [0.05, 0.15] }),
    ...grid("vintage", { kParamVintageScale: [25, 70, 100], kParamSize: [0, 50, 90] }),
    ...grid("vintage", { kParamVintageScale: [25, 70, 100], kParamDelay: [0, 100] }),
    ...grid("vintage", { kParamSize: [0, 50, 90], kParamPreDelay: [0.1] }),
    ...grid("vintage", { kParamSize: [0, 50, 90], kParamDelay: [0] }),
    // Delay matters only near 0: finer steps there.
    ...[2, 5].map((v) => ["vintage", { kParamDelay: v }]),
    ...grid("vintage", { kParamVintageScale: [25, 100], kParamDelay: [2, 5, 10] }),
    ...grid("vintage", { kParamSize: [0, 50, 90], kParamDelay: [2, 5, 10] }),
    ...grid("vintage", { kParamVintageScale: [0, 10], kParamSize: [0, 50, 90] }),
    ...grid("vintage", { kParamVintageScale: [0, 25, 50, 85], kParamMode: [0, 50] }),
    ...grid("abyss", { kParamMode: [0, 25, 50, 75, 100], kParamDelay: [0, 20, 100, 250] }),
    ...grid("abyss", { kParamMode: [0, 25, 50, 100], kParamSize: [0, 25, 50, 75, 100] }),
    ...grid("abyss", { kParamDelay: [0, 20, 100], kParamSize: [0, 25, 50, 75, 100] }),
    ...grid("abyss", { kParamMode: [0, 50, 100], kParamPreDelay: [0.02, 0.1, 0.3] }),
    ...grid("abyss", { kParamSize: [10, 50, 90], kParamPreDelay: [0.1, 0.3] }),
    ...grid("abyss", { kParamDelay: [0, 100], kParamPreDelay: [0.1, 0.3] }),
    ...grid("abyss", { kParamWidth: [0, 50, 100], kParamSize: [25, 50, 75] }),
    ...grid("abyss", { kParamWidth: [50, 100], kParamMode: [0, 50, 100] }),
  ].map(([t, q]) => [`fxrv8-${t}-${Object.entries(q).map(([k, v]) => `${REVERB_CONTROL[k]}${v}`).join("-")}`,
    [...CLICK, ["Env0", "kParamSustain", 1e-3], ...rack(fx("FXReverb", { kParamWet: 100, kParamType: `k${t[0].toUpperCase()}${t.slice(1)}`, ...q }))]])),
  // Vintage's direct copy on a sustained saw (fxrv11-): output before the tail against the dry.
  ...Object.fromEntries([["default", {}], ["vscale50", { kParamVintageScale: 50 }], ["vscale67-mode0", { kParamVintageScale: 67, kParamMode: 0 }], ["predelay0_02", { kParamPreDelay: 0.02 }]]
    .map(([n, q]) => [`fxrv11-vintage-${n}`, rack(fx("FXReverb", { kParamWet: 100, kParamType: "kVintage", ...q }))])),
  // Factory reverb modules on the same click at 100 % wet (fxrvf-): checks how controls combine.
  ...factoryReverbs(process.env.LUMEN_FACTORY_PRESETS),
  // Reverb with every main control set explicitly, as factory presets store them (fxrv2-).
  "fxrv2-dry": CLICK,
  ...Object.fromEntries(["kHall", "kVintage", "kAbyss", "kSpace", null].flatMap((t) => [[30, 35], [60, 35], [30, 80]].map(([size, decay]) => [`fxrv2-${t ? t.slice(1).toLowerCase() : "untyped"}-s${size}-d${decay}`, [...CLICK, ...rack(fx("FXReverb", { kParamWet: 100, kParamSize: size, kParamFeedback: decay, kParamDelay: 20, kParamFreqB: 50, kParamFreq: 10, ...(t ? { kParamType: t } : {}) }))]]))),
  // FX filter responses (fxr-ff-): the FX filter against the voice filter's measured models.
  ...Object.fromEntries([
    ...["L12", "MgL12", "H12", "B12", "L24", "LH12", "CombP", "Phase24P"].flatMap((t) => [10, 60].map((q) => [`ff-${t}-r${q}`, { kParamType: t, kParamFreq: 0.5, kParamReso: q }])),
    ["ff-L12-f03", { kParamType: "L12", kParamFreq: 0.3 }], ["ff-L12-f07", { kParamType: "L12", kParamFreq: 0.7 }],
    ["ff-L12-drive50", { kParamType: "L12", kParamFreq: 0.5, kParamDrive: 50 }],
    ["ff-L12-stereo0", { kParamType: "L12", kParamFreq: 0.5, kParamStereo: 0 }],
    ["ff-L12-level1", { kParamType: "L12", kParamFreq: 0.5, kParamLevelOut: 1 }],
    ["ff-default", {}],
  ].map(([n, params]) => [`fxr-${n}`, rack(fx("FXFilter", params))])),
  // Compressor static curve (fxc-): a sine at 12 input levels (FX Utility stages set the
  // level; `-dry` renders are the same chain without the compressor) through each setting.
  ...Object.fromEntries(["sine", "square"].flatMap((shape) => COMP_LEVELS.flatMap(([lv, stages]) => [["dry", null], ...Object.entries(shape === "sine" ? COMP_SETTINGS : { limit: COMP_SETTINGS["limit-0.5"] })].map(([n, params]) => [`fxc-${shape}-${lv}-${n}`, [...SUB_SINE, ...(shape === "square" ? [["Oscillator4.SubOsc4", "kParamShape", "kSquare"]] : []), ...rack(...stages.map((l) => fx("FXUtils", { kParamLevelOut: l })), ...(params ? [fx("FXComp", params)] : []))]])))),
  // Compressor time constants (fxct-): the sub, +12 dB, steps up at note on and down to
  // sustain 0.05 after 0.8 s (amp envelope hold); `-dry` has no compressor.
  ...Object.fromEntries([["dry", null], ...[0.1, 1, 10, 50, 200, 1000].map((a) => [`attack-${a}`, { kParamAttack: a, kParamRelease: 100 }]), ...[5, 50, 100, 300, 1000].map((r) => [`release-${r}`, { kParamAttack: 0.1, kParamRelease: r }])].map(([n, times]) => [`fxct-${n}`, [...SUB_SINE, ["Env0", "kParamAttack", 0], ["Env0", "kParamHold", 0.8], ["Env0", "kParamDecay", 0], ["Env0", "kParamSustain", 0.05], ...rack(fx("FXUtils", { kParamLevelOut: 1 }), ...(times ? [fx("FXComp", { kParamThresh: 0.5, kParamRatio: 1000000, ...times })] : []))]])),
  // The same steps through the ratio path (ratio 4), plus makeup and ratio-switch points (fxcr-).
  ...Object.fromEntries([["dry", null], ...[0.1, 1, 10, 100, 1000].map((a) => [`attack-${a}`, { kParamAttack: a, kParamRelease: 100 }]), ...[1, 10, 100, 1000].map((r) => [`release-${r}`, { kParamAttack: 1, kParamRelease: r }])].map(([n, times]) => [`fxcr-step-${n}`, [...SUB_SINE, ["Env0", "kParamAttack", 0], ["Env0", "kParamHold", 0.8], ["Env0", "kParamDecay", 0], ["Env0", "kParamSustain", 0.05], ...rack(fx("FXUtils", { kParamLevelOut: 1 }), ...(times ? [fx("FXComp", { kParamThresh: 0.5, kParamRatio: 4, ...times })] : []))]])),
  // Multiband detector on a square (its harmonics spread over the bands).
  ...Object.fromEntries(COMP_LEVELS.flatMap(([lv, stages]) => [["dry", null], ["default", {}], ["fast", MB_SETTINGS.fast]].map(([n, params]) => [`fxmbsq-${lv}-${n}`, [...SUB_SINE, ["Oscillator4.SubOsc4", "kParamShape", "kSquare"], ...rack(...stages.map((l) => fx("FXUtils", { kParamLevelOut: l })), ...(params ? [fx("FXComp", { kParamMultiband: 1, ...params })] : []))]]))),
  // The same steps through the multiband compressor at its defaults.
  ...Object.fromEntries([["dry", null], ...[0.1, 1, 10, 100, 1000].map((a) => [`attack-${a}`, { kParamAttack: a, kParamRelease: 100 }]), ...[1, 10, 100, 1000].map((r) => [`release-${r}`, { kParamAttack: 1, kParamRelease: r }])].map(([n, times]) => [`fxmbt-${n}`, [...SUB_SINE, ["Env0", "kParamAttack", 0], ["Env0", "kParamHold", 0.8], ["Env0", "kParamDecay", 0], ["Env0", "kParamSustain", 0.05], ...rack(fx("FXUtils", { kParamLevelOut: 1 }), ...(times ? [fx("FXComp", { kParamMultiband: 1, ...times })] : []))]])),
  "fxcr-quiet-dry": [...SUB_SINE, ...rack(fx("FXUtils", { kParamLevelOut: 0.05 }))],
  ...Object.fromEntries([0.5, 1, 1.58, 2.56, 3, 6, 12, 20, 31].map((m) => [`fxcr-quiet-makeup-${m}`, [...SUB_SINE, ...rack(fx("FXUtils", { kParamLevelOut: 0.05 }), fx("FXComp", { kParamMakeup: m }))]])),
  "fxcr-loud-dry": [...SUB_SINE, ...rack(fx("FXUtils", { kParamLevelOut: 1 }))],
  ...Object.fromEntries([20, 50, 100, 1000, 10000, 100000, 999999].map((r) => [`fxcr-loud-ratio-${r}`, [...SUB_SINE, ...rack(fx("FXUtils", { kParamLevelOut: 1 }), fx("FXComp", { kParamThresh: 0.5, kParamRatio: r }))]])),
  ...Object.fromEntries([[0.1, 10], [1, 100], [10, 1000], [50, 500]].map(([a, r]) => [`fxcr-loud-times-${a}-${r}`, [...SUB_SINE, ...rack(fx("FXUtils", { kParamLevelOut: 1 }), fx("FXComp", { kParamThresh: 0.5, kParamRatio: 4, kParamAttack: a, kParamRelease: r }))]])),
  "fxcr-loud-wet-50-uncomp": [...SUB_SINE, ...rack(fx("FXUtils", { kParamLevelOut: 1 }), fx("FXComp", { kParamThresh: 0.5, kParamRatio: 1000000, kParamWet: 50, kParamCompensatedWetDry: 0 }))],
  "fxcr-loud-square-ratio-4": [...SUB_SINE, ["Oscillator4.SubOsc4", "kParamShape", "kSquare"], ...rack(fx("FXUtils", { kParamLevelOut: 1 }), fx("FXComp", { kParamThresh: 0.5, kParamRatio: 4 }))],
  "fxcr-loud-square-dry": [...SUB_SINE, ["Oscillator4.SubOsc4", "kParamShape", "kSquare"], ...rack(fx("FXUtils", { kParamLevelOut: 1 }))],
  // Multiband compressor static curves (fxmb-): the fxc- levels through each multiband
  // setting; rendered at several notes so the sine falls in each band.
  ...Object.fromEntries(COMP_LEVELS.flatMap(([lv, stages]) => [["dry", null], ...Object.entries(MB_SETTINGS)].map(([n, params]) => [`fxmb-${lv}-${n}`, [...SUB_SINE, ...rack(...stages.map((l) => fx("FXUtils", { kParamLevelOut: l })), ...(params ? [fx("FXComp", { kParamMultiband: 1, ...params })] : []))]]))),
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
