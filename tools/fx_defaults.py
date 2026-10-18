"""Measure FX module parameter ranges and defaults through state readback.

Serum 2 clamps values on load (so ±1e6 reveals min/max) and omits any field equal to
its default (so a candidate that disappears on readback is the default). Every
module type sits in a rack at once, so one state load tests one candidate for
every parameter. Writes serum_fx_params.json.
"""
import json, re, struct, sys, time
sys.argv = sys.argv[:1]
import param_map as pm
from serum_render import juce_b64_decode, juce_b64_encode, container_read, container_write, cbor_top_map, cbor_map_bytes


def enc(v):
    """Minimal CBOR encoder for str/float/int/dict/list (floats as float64)."""
    if isinstance(v, str):
        b = v.encode(); return head(3, len(b)) + b
    if isinstance(v, bool):
        return bytes([0xF5 if v else 0xF4])
    if isinstance(v, int):
        return head(0, v) if v >= 0 else head(1, -1 - v)
    if isinstance(v, float):
        return b"\xfb" + struct.pack(">d", v)
    if isinstance(v, list):
        return head(4, len(v)) + b"".join(enc(x) for x in v)
    if isinstance(v, dict):
        return head(5, len(v)) + b"".join(enc(k) + enc(x) for k, x in sorted(v.items(), key=lambda kv: kv[0].encode()))
    raise TypeError(v)


def head(major, n):
    m = major << 5
    if n < 24: return bytes([m | n])
    if n < 256: return bytes([m | 24, n])
    if n < 65536: return bytes([m | 25]) + n.to_bytes(2, "big")
    return bytes([m | 26]) + n.to_bytes(4, "big")


def state_with(base_state, racks):
    xml = base_state[8:].decode()
    comp = re.search(r"<IComponent>(.*?)</IComponent>", xml).group(1)
    meta, version, cbor = container_read(juce_b64_decode(comp))
    entries = [(k, kr, enc(racks[k]) if k in racks else v) for k, kr, v in cbor_top_map(cbor)]
    blob = container_write(meta, version, cbor_map_bytes(entries))
    body = xml.replace(comp, juce_b64_encode(blob)).encode()
    return b"VC2!" + struct.pack("<I", len(body)) + body


def rack(modules):
    return {"FX": [{name: {"plainParams": params if params else "default"}, "type": t} for name, t, params in modules], "displayName": "", "plainParams": "default"}


def main():
    cand = json.load(open("fx-candidates.json"))
    groups = [{"FXRack0": [n for n in cand if not n.startswith("FXSplit")]}, {"FXRack0": ["FXSplit"], "FXRack1": ["FXSplit3"], "FXRack2": ["FXSplitMS"]}]
    serum = pm.load_plugin(pm.SERUM, plugin_name="Serum 2")
    base = serum.raw_state
    result = {n: {"type": cand[n]["type"], "params": {k: {"default": None, "min": None, "max": None, "readback": {}} for k in cand[n]["params"]}} for n in cand}
    t = time.time()

    def run(group, pick):
        racks = {}
        for rack_name, names in group.items():
            mods = []
            for n in names:
                params = {}
                for k, c in cand[n]["params"].items():
                    v = pick(n, k, c)
                    if v is not None:
                        params[k] = v
                mods.append((n, cand[n]["type"], params))
            racks[rack_name] = rack(mods)
        serum.raw_state = state_with(base, racks)
        snap = pm.snapshot(serum)
        back = {}
        for rack_name, names in group.items():
            for i, n in enumerate(names):
                prefix = f"{rack_name}.FX[{i}].{n}.plainParams."
                back[n] = {k[len(prefix):]: v for k, v in snap.items() if k.startswith(prefix)}
        return racks, back

    for group in groups:
        # Ranges: clamped readback of huge values.
        for label, big in (("max", 1e6), ("min", -1e6)):
            # Module counts regroup the rack; huge values crash the plugin, so skip them.
            racks, back = run(group, lambda n, k, c, big=big: None if c["texts"] or k.startswith("kParamModuleCount") else big)
            for n in back:
                for k, v in back[n].items():
                    if k in result[n]["params"]:
                        result[n]["params"][k][label] = v
        # Defaults: candidates that vanish on readback.
        rounds = max(len(c["values"]) + len(c["texts"]) for n in group.values() for m in n for c in cand[m]["params"].values())
        for i in range(rounds):
            def pick(n, k, c, i=i):
                if result[n]["params"][k]["default"] is not None:
                    return None
                options = c["texts"] + c["values"]
                if k.startswith("kParamModuleCount"):
                    options = [v for v in options if isinstance(v, (int, float)) and 0 <= v <= 3 and float(v).is_integer()]
                return options[i] if i < len(options) else None
            racks, back = run(group, pick)
            for rack_name, names in group.items():
                for j, n in enumerate(names):
                    sent = racks[rack_name]["FX"][j][n]["plainParams"]
                    if sent == "default":
                        continue
                    for k, v in sent.items():
                        if k not in back[n]:
                            result[n]["params"][k]["default"] = v
                        else:
                            result[n]["params"][k]["readback"][str(v)] = back[n][k]
        print(f"group done in {time.time() - t:.0f}s", flush=True)
    for n in result:
        for k, r in result[n]["params"].items():
            r["readback"] = dict(list(r["readback"].items())[:12])
    json.dump(result, open("serum_fx_params.json", "w"), indent=1)
    found = sum(r["default"] is not None for n in result for r in result[n]["params"].values())
    total = sum(len(result[n]["params"]) for n in result)
    print(f"defaults found for {found}/{total} parameters")


if __name__ == "__main__":
    main()
