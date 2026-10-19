"""Generate crates/engine/src/params/serum2.rs from the measured parameter data.

Inputs (data/):
  serum2-param-order.json  module -> parameter names in Serum's enum order
                           (index = destModuleParamID in mod slots)
  serum2-params.json       per preset field: host name, fitted curve, default
  serum2-choices.json      per choice field: ordered options (stored name, display)

Run from the repo root: python3 tools/gen_params.py
"""
import json, re

ORDER = json.load(open("data/serum2-param-order.json"))
FIELDS = json.load(open("data/serum2-params.json"))
# plainParams keys seen in factory presets but neither host parameters nor enumerated.
PRESET_KEYS = json.load(open("data/serum2-preset-keys.json"))
# FX modules: ranges and defaults measured by state readback (tools/fx_defaults*.py).
FX = json.load(open("data/serum2-fx-params.json"))
FILTER_TYPES = None  # filled from VoiceFilter.kParamType options
FX_CHOICES = {
    ("FXDistortion", "kParamMode"): ["kTube", "kSoftClip", "kHardClip", "kDiode1", "kDiode2", "kLinFold", "kSinFold", "kZeroSquare", "kDownsample", "kAsym", "kRectify", "kXShaper", "kXShaperAsym", "kSineShaper", "kStompBox", "kTapeSat", "kOverdrive", "kSoftSat"],
    ("FXReverb", "kParamType"): ["kHall", "kVintage", "kAbyss", "kSpace"],
}
# Defaults the readback search could not pin down (no exact float match), from
# render probes where noted. (module, param): (default, min, max[, evidence])
FX_ASSUMED = {
    ("FXBode", "kParamRange"): (1000.0, 0.1, 5000.0),
    ("FXDelay", "kParamFreq"): (850.0, 40.0, 18000.0),
    # The default renders exactly like 0.0625 (a quarter note; delay probes).
    ("FXDelay", "kParamTimeL"): (0.0625, 0.001, 0.501, "measured"),
    ("FXDelay", "kParamTimeR"): (0.0625, 0.001, 0.501, "measured"),
    # Measured: the default compressor barely changes level, so the default ratio is ~1:1.
    ("FXComp", "kParamRatio"): (1.0, 1.0, 1000000.0),
    ("FXComp", "kParamAttack"): (10.0, 0.1, 1000.0),
    ("FXComp", "kParamRelease"): (100.0, 0.1, 1000.0),
    ("FXComp", "kParamXoverLow"): (300.0, 20.0, 5000.0),
    ("FXComp", "kParamXoverHi"): (2500.0, 300.0, 18000.0),
    ("FXComp", "kParamDeadband1"): (11.6, 3.0, 24.0),
    ("FXComp", "kParamDeadband2"): (5.3, 3.0, 24.0),
    ("FXEQ", "kParamFreq1"): (200.0, 21.5332, 20000.0),
    ("FXEQ", "kParamFreq2"): (4000.0, 21.5332, 20000.0),
    ("FXSplit", "kParamFreq"): (300.0, 21.5332, 20000.0),
    ("FXSplit3", "kParamFreq"): (200.0, 21.5332, 20000.0),
    ("FXSplit3", "kParamFreq2"): (2000.0, 21.5332, 20000.0),
}
FX_FREQ = re.compile(r"Freq|Xover|HPF|LPF|Filt$|Range")

# Natural defaults for preset-only ranges whose default never appears in presets.
PRESET_DEFAULTS = {"kParamKeyZoneMax": 127, "kParamKeyZoneMin": 0, "kParamVelocityZoneMax": 127, "kParamVelocityZoneMin": 0}
CHOICES = json.load(open("data/serum2-choices.json"))

# Section path pattern -> (module name, instance count). Order defines registry order.
MODULES = [
    ("Oscillator#.plainParams", "Oscillator", 5),
    ("Oscillator#.WTOsc#.plainParams", "WTOsc", 3),
    # Sub-engines for the other oscillator types (host parameters appear only when
    # an oscillator uses the type, so these come from Serum's enum order and presets).
    ("Oscillator#.SampleOsc#.plainParams", "SampleOsc", 3),
    ("Oscillator#.MultiSampleOsc#.plainParams", "MultiSampleOsc", 3),
    ("Oscillator#.GranularOsc#.plainParams", "GranularOsc", 3),
    ("Oscillator#.SpectralOsc#.plainParams", "SpectralOsc", 3),
    # Single-instance modules inside fixed oscillator slots.
    ("Oscillator3.NoiseOsc3.plainParams", "NoiseOsc", 1),
    ("Oscillator4.SubOsc4.plainParams", "SubOsc", 1),
    ("VoiceFilter#.plainParams", "VoiceFilter", 2),
    ("Env#.plainParams", "Env", 4),
    ("LFO#.plainParams", "LFO", 10),
    ("Macro#.plainParams", "Macro", 8),
    ("Global#.plainParams", "Global", 1),
    ("RoutingSlot#.plainParams", "RoutingSlot", 7),
    ("VoicePanel#.plainParams", "VoicePanel", 1),
    ("ModSlot#.plainParams", "ModSlot", 64),
    ("Arp#.plainParams", "Arp", 1),
    ("ClipPlayer#.plainParams", "ClipPlayer", 1),
    ("PitchQuantizer#.plainParams", "PitchQuantizer", 1),
]

# Preset-only parameters (not host parameters), with defaults derived from the
# factory presets: (module, name, curve, default, evidence)
EXTRA = [
    # Slot 0 (OSC A) defaults to the filter; the others to the master bus (factory presets).
    ("RoutingSlot", "kParamRoutingDest", ("choice", ["kRoutingDestFilter", "kRoutingDestMaster", "kRoutingDestDirect", "kRoutingDestNone"]), [0, 1, 1, 1, 1, 1, 1], "presets"),
    ("Global", "kParamPolyCount", ("linear", 1, 32, True), 8, "assumed"),
    ("Global", "kParamGlobalTuning", ("linear", 400, 480, False), 440, "presets"),
    # Factory presets store 0 and 2 explicitly, so the default is 1; renders show 0 runs
    # filters at 2× and 1 (like 2) at 4× (docs/CALIBRATION.md).
    ("Global", "kParamOversampling", ("linear", 0, 2, True), 1, "presets"),
    ("Global", "kParamLimitSameNotePolyphony", ("toggle",), 0, "assumed"),
    ("Oscillator", "kParamType", ("choice", ["kOsc_WT", "kOsc_Sample", "kOsc_MultiSample", "kOsc_Granular", "kOsc_Spectral"]), 0, "presets"),
    ("Oscillator", "kParamDetuneMode", ("choice", ["kDetuneLinear", "kDetuneSuper", "kDetuneExp", "kDetuneInv", "kDetuneRandom"]), 0, "binary name table"),
    ("Oscillator", "kParamUnisonRange", ("linear", 0, 48, False), 2, "assumed"),
    ("Oscillator", "kParamPitchMode", ("choice", ["Semitones", "Harmonics", "Ratio"]), 0, "presets"),
    ("VoiceFilter", "kParamKeyTrack", ("toggle",), 0, "presets"),
    ("VoiceFilter", "kParamPad", ("toggle",), 0, "presets"),
    ("Env", "kParamBeatSync", ("toggle",), 0, "presets"),
    ("Env", "kParamLegatoInverted", ("toggle",), 0, "presets"),
    ("LFO", "kParamMode", ("choice", ["Trig", "Free", "Envelope"]), 0, "presets"),
    # Presets omit the default, the ordinary drawn-curve LFO, so its stored name is unknown;
    # "Curve" is Lumen's. "Path" (stored explicitly) is the XY path LFO (lfo/path probes).
    ("LFO", "kParamType", ("choice", ["Curve", "Path", "RandomSH", "Lorenz", "Rossler"]), 0, "presets"),
    ("LFO", "kParamBeatSync", ("toggle",), 1, "presets"),
    ("LFO", "kParamDotted", ("toggle",), 0, "presets"),
    ("LFO", "kParamTriplets", ("toggle",), 0, "presets"),
    ("LFO", "kParamRate10x", ("toggle",), 0, "presets"),
    ("LFO", "kParamMono", ("toggle",), 0, "presets"),
    ("LFO", "kParamAnchored", ("toggle",), 1, "presets"),
    ("LFO", "kParamDirection", ("linear", 0, 2, True), 0, "presets"),
    ("LFO", "kParamDefaultMode", ("toggle",), 1, "presets"),
    ("LFO", "kParamGridX", ("linear", 1, 32, True), 8, "assumed"),
    ("LFO", "kParamGridY", ("linear", 1, 32, True), 8, "assumed"),
    ("ModSlot", "kParamBipolar", ("toggle",), 0, "presets"),
    ("ModSlot", "kParamBypass", ("toggle",), 0, "presets"),
    ("ModSlot", "kParamAuxInverted", ("toggle",), 0, "presets"),
    ("ModSlot", "kParamCurveIn", ("linear", -100, 100, False), 0, "presets"),
    ("ModSlot", "kParamCurveOut", ("linear", -100, 100, False), 0, "presets"),
    ("ModSlot", "kParamSmoothRise", ("linear", 0, 100, False), 0, "presets"),
    ("ModSlot", "kParamSmoothFall", ("linear", 0, 100, False), 0, "presets"),
]

# Stored name of a choice's default when Serum omits it (it is never written).
DEFAULT_CHOICE_NAME = {"kParamWarpMenu": "kNoWarp", "kParamWarpMenu2": "kNoWarp"}


def clean(v):
    if v is None:
        return None
    v = float(v)
    if abs(v - round(v)) < 2e-5:
        return float(round(v))
    return float(f"{v:.6g}")


def interp(points, x):
    pts = sorted(points)
    if x <= pts[0][0]:
        return pts[0][1]
    for (x0, y0), (x1, y1) in zip(pts, pts[1:]):
        if x0 <= x <= x1:
            return y0 + (y1 - y0) * (x - x0) / (x1 - x0) if x1 > x0 else y0
    return pts[-1][1]


def module_of(field):
    path, name = field.rsplit(".", 1)
    pattern = re.sub(r"\d+", "#", path)
    instance = int(re.findall(r"(\d+)", path)[-1]) if re.findall(r"\d+", path) else 0
    for pat, module, count in MODULES:
        if pat == path:
            return module, 0, name
        if pat == pattern:
            return module, instance, name
    return None, None, name


def choice_options(field, entry, default_text=None):
    data = CHOICES.get(field) or CHOICES.get(re.sub(r"(Oscillator|WTOsc|VoiceFilter)\d", lambda m: m.group(1) + "0", field))
    names = []
    if data:
        for _x, stored, _display in data["options"]:
            if stored and stored not in names:
                names.append(stored)
        first_stored = data["options"][0][1]
        if first_stored is None:
            pname = field.rsplit(".", 1)[1]
            fallback = DEFAULT_CHOICE_NAME.get(pname)
            if fallback and fallback not in names:
                names.insert(0, fallback)
        displays = {}
        for _x, stored, display in data["options"]:
            key = stored or names[0]
            displays.setdefault(key, display)
        shown = [displays.get(n, n) for n in names]
        # The default's own field is never written, so identify it by its display text.
        default_index = shown.index(default_text) if default_text in shown else 0
        return names, shown, default_index
    return entry.get("options", []), entry.get("options", []), 0


def curve_of(entry):
    k = entry["kind"]
    if k == "linear":
        return ("linear", clean(entry["min"]), clean(entry["max"]), bool(entry.get("integer")))
    if k == "power":
        return ("power", clean(entry["min"]), clean(entry["max"]), entry["k"])
    if k == "exp":
        return ("exp", clean(entry["min"]), clean(entry["max"]))
    if k == "toggle":
        return ("toggle",)
    if k in ("steps", "table"):
        pts = entry["points"]
        ys = [y for _, y in pts]
        integer = all(abs(y - round(y)) < 1e-6 for y in ys)
        return ("linear", clean(min(ys)), clean(max(ys)), integer)
    # Unknown: Serum stores the normalized value itself (e.g. filter cutoff).
    return ("linear", 0.0, 1.0, False)


def default_of(entry, curve):
    d = entry.get("default")
    if d is None and entry["kind"] in ("steps", "table"):
        d = interp(entry["points"], entry["default_raw"])
        if curve[0] == "linear" and curve[3]:
            d = round(d)
    if d is None and entry["kind"] == "unknown":
        d = entry["default_raw"]
    return clean(d) if d is not None else 0.0


def rust_curve(c):
    if c[0] == "linear":
        return f"Curve::Linear {{ min: {f32(c[1])}, max: {f32(c[2])}, integer: {str(c[3]).lower()} }}"
    if c[0] == "power":
        return f"Curve::Power {{ min: {f32(c[1])}, max: {f32(c[2])}, k: {f32(c[3])} }}"
    if c[0] == "exp":
        return f"Curve::Exp {{ min: {f32(c[1])}, max: {f32(c[2])} }}"
    if c[0] == "toggle":
        return "Curve::Toggle"
    raise ValueError(c)


def f32(v):
    s = repr(float(v))
    return s if ("." in s or "e" in s) else s + ".0"


def main():
    per_module = {m: {} for _, m, _ in MODULES}
    for entry in FIELDS:
        module, instance, name = module_of(entry["field"])
        if module is None:
            continue
        slot = per_module[module].setdefault(name, {"host": None, "curve": None, "defaults": {}, "options": [], "displays": [], "evidence": "measured"})
        if entry["kind"] == "choice":
            names, displays, di = choice_options(entry["field"], entry, entry.get("default_text"))
            slot["curve"] = ("choice", names)
            slot["options"], slot["displays"] = names, displays
            slot["defaults"][instance] = float(di)
        else:
            curve = curve_of(entry)
            slot["curve"] = slot["curve"] or curve
            slot["defaults"][instance] = default_of(entry, curve)
        if slot["host"] is None or instance == 0:
            # Host name of the first instance; others differ only in their index.
            slot["host"] = entry["host"]
    for module, name, curve, default, evidence in EXTRA:
        if name in per_module[module]:
            continue
        slot = {"host": None, "curve": curve, "defaults": {}, "options": [], "displays": [], "evidence": evidence}
        if curve[0] == "choice":
            slot["options"], slot["displays"] = curve[1], curve[1]
            default = 0 if default is None else default
        slot["default_all"] = default if isinstance(default, list) else [float(default if default is not None else 0)]
        per_module[module][name] = slot

    for k in PRESET_KEYS:
        if k["name"] in per_module[k["module"]]:
            continue
        if k["texts"]:
            # The default option is never stored, so an explicit unset option stands for it.
            options = ["(unset)"] + k["texts"]
            slot = {"curve": ("choice", options), "options": options, "displays": options, "default_all": [0.0]}
        elif k["min"] == k["max"]:
            # Only the non-default value is ever stored.
            slot = {"curve": ("toggle",), "options": [], "displays": [], "default_all": [0.0 if k["max"] else 1.0]}
        else:
            lo, hi = k["min"], k["max"]
            d = PRESET_DEFAULTS.get(k["name"], 0.0)
            lo, hi = min(lo, d), max(hi, d)
            slot = {"curve": ("linear", clean(lo), clean(hi), k["integer"]), "options": [], "displays": [], "default_all": [float(d)]}
        slot.update({"host": None, "defaults": {}, "evidence": "presets"})
        per_module[k["module"]][k["name"]] = slot

    fx_modules = []
    filter_types = per_module["VoiceFilter"]["kParamType"]["options"]
    for module, info in FX.items():
        order = list(ORDER.get(module, []))
        names = order + [k for k in info["params"] if k not in order]
        slots = {}
        for name in names:
            r = info["params"].get(name, {"default": None, "min": None, "max": None})
            default, lo, hi = r["default"], r["min"], r["max"]
            evidence = "measured"
            if (module, name) in FX_ASSUMED:
                default, lo, hi, *rest = FX_ASSUMED[(module, name)]
                evidence = rest[0] if rest else "assumed"
            choice = FX_CHOICES.get((module, name)) or (filter_types if (module, name) == ("FXFilter", "kParamType") else None)
            if choice:
                idx = int(default) if isinstance(default, (int, float)) else (choice.index(default) if default in choice else 0)
                slots[name] = {"curve": ("choice", choice), "options": choice, "displays": choice, "default_all": [float(idx)], "evidence": evidence}
                continue
            if name.startswith("kParamModuleCount"):
                lo, hi = 0, 32
            # A bound equal to the default was omitted by the readback; it is the default.
            lo = default if lo is None else lo
            hi = default if hi is None else hi
            if lo is None or hi is None or default is None:
                lo, hi, default, evidence = 0.0, 1.0, 0.0, "assumed"
            lo, hi = min(lo, hi, default), max(lo, hi, default)
            if lo == 0 and hi == 1 and float(default).is_integer() and name not in ("kParamFreq", "kParamLevelOut", "kParamThresh", "kParamDepth2", "kParamX", "kParamY"):
                curve = ("toggle",)
            elif r.get("default_curve") == "power2" or name == "kParamPreDelay":
                curve = ("power", clean(lo), clean(hi), 2.0)
            elif FX_FREQ.search(name) and lo > 0:
                curve = ("exp", clean(lo), clean(hi))
            else:
                integer = name in ("kParamUnison", "kParamNumPoles", "kParamNumStages", "kParamMode", "kParamType1", "kParamType2", "kParamPrePost") or name.startswith("kParamModuleCount")
                curve = ("linear", clean(lo), clean(hi), integer)
            slots[name] = {"curve": curve, "options": [], "displays": [], "default_all": [clean(default)], "evidence": evidence}
        for slot in slots.values():
            slot.update(host=None, defaults={})
        per_module[module] = slots
        fx_modules.append((module, names))

    lines = [
        "// @generated by tools/gen_params.py from data/serum2-*.json. Do not edit by hand.",
        "//",
        "// Parameter order follows Serum 2's per-module enums, so a parameter's index is",
        "// the `destModuleParamID` stored in mod slots. Curves and defaults were measured",
        "// through the host parameter interface; `evidence` marks the rest.",
        "",
        "use super::{Curve, Evidence, ModuleSpec, ParamSpec};",
        "",
    ]
    registry = []
    all_modules = [(m, c, None) for _, m, c in MODULES] + [(m, 0, names) for m, names in fx_modules]
    for module, count, fixed_names in all_modules:
        params = per_module[module]
        order = list(ORDER.get(module, []))
        names = fixed_names or order + [n for n in sorted(params) if n not in order]
        const = re.sub(r"(?<!^)(?=[A-Z][a-z])", "_", module).upper()
        # FX modules live in rack lists; they have no fixed section and 0 static instances.
        section = next((pat for pat, m, _ in MODULES if m == module), "").rsplit(".plainParams", 1)[0]
        lines.append(f"pub const {const}: ModuleSpec = ModuleSpec {{")
        lines.append(f'    name: "{module}",')
        lines.append(f'    section: "{section}",')
        lines.append(f"    instances: {count},")
        lines.append(f"    automatable: {len(order)},")
        lines.append("    params: &[")
        for name in names:
            slot = params.get(name)
            if slot is None:
                # Enumerated by Serum but never seen in host parameters or presets.
                lines.append(f'        ParamSpec {{ name: "{name}", host: None, curve: Curve::Linear {{ min: 0.0, max: 1.0, integer: false }}, defaults: &[0.0], options: &[], displays: &[], evidence: Evidence::Unknown }},')
                continue
            curve = slot["curve"]
            if "default_all" in slot:
                defaults = slot["default_all"]
            else:
                ds = slot["defaults"]
                defaults = [ds.get(i, ds.get(min(ds), 0.0)) for i in range(count)]
                if len(set(defaults)) == 1:
                    defaults = defaults[:1]
            curve_rs = "Curve::Choice" if curve[0] == "choice" else rust_curve(curve)
            host = f'Some("{slot["host"]}")' if slot["host"] else "None"
            opts = ", ".join(f'"{o}"' for o in slot["options"])
            disp = ", ".join('"' + d.replace('"', '\\"') + '"' for d in slot["displays"])
            evidence = {"measured": "Measured", "presets": "Presets", "assumed": "Assumed", "binary name table": "NameTable"}[slot["evidence"]]
            if count == 0 and len(defaults) > 1:
                defaults = defaults[:1]
            lines.append(f'        ParamSpec {{ name: "{name}", host: {host}, curve: {curve_rs}, defaults: &[{", ".join(f32(d) for d in defaults)}], options: &[{opts}], displays: &[{disp}], evidence: Evidence::{evidence} }},')
        lines.append("    ],")
        lines.append("};")
        lines.append("")
        registry.append(const)
    lines.append(f"pub const ALL: [&ModuleSpec; {len(registry)}] = [{', '.join('&' + r for r in registry)}];")
    open("crates/engine/src/params/serum2.rs", "w").write("\n".join(lines) + "\n")
    total = sum(len(v) for v in per_module.values())
    print(f"{len(registry)} modules, {total} parameters")


if __name__ == "__main__":
    main()
