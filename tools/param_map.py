"""Map every Serum 2 host parameter to the preset field it writes, with plain values at
normalized 0, .25, .5, .75, 1 and the default. Writes serum_param_map.json."""
import json, re, struct, sys, time
sys.argv = sys.argv[:1]
from serum_render import load_plugin, SERUM, juce_b64_decode, container_read

def cbor_decode(b, i=0):
    ib = b[i]; major, info = ib >> 5, ib & 31; i += 1
    def arg(i):
        if info < 24: return info, i
        n = {24: 1, 25: 2, 26: 4, 27: 8}[info]; return int.from_bytes(b[i:i + n], "big"), i + n
    if major == 7:
        if info == 20: return False, i
        if info == 21: return True, i
        if info == 22: return None, i
        if info == 26: return struct.unpack(">f", b[i:i + 4])[0], i + 4
        if info == 27: return struct.unpack(">d", b[i:i + 8])[0], i + 8
        if info == 25:
            h = int.from_bytes(b[i:i + 2], "big"); e, f = (h >> 10) & 31, h & 1023
            v = (f / 1024 * 2 ** -14) if e == 0 else (1 + f / 1024) * 2 ** (e - 15)
            return (-v if h & 0x8000 else v), i + 2
    n, i = arg(i)
    if major == 0: return n, i
    if major == 1: return -1 - n, i
    if major == 2: return b[i:i + n].hex(), i + n
    if major == 3: return b[i:i + n].decode(), i + n
    if major == 4:
        out = []
        for _ in range(n): v, i = cbor_decode(b, i); out.append(v)
        return out, i
    if major == 5:
        out = {}
        for _ in range(n):
            k, i = cbor_decode(b, i); v, i = cbor_decode(b, i); out[k] = v
        return out, i
    if major == 6: return cbor_decode(b, i)

def flat(v, path, out):
    if isinstance(v, dict):
        for k, c in v.items(): flat(c, f"{path}.{k}" if path else k, out)
    elif isinstance(v, list) and path.endswith(".FX"):
        # FX rack module lists: index them so module parameters are addressable.
        for i, c in enumerate(v): flat(c, f"{path}[{i}]", out)
    elif not isinstance(v, list):
        out[path] = v
    return out

def snapshot(serum):
    # Parameter changes reach the VST3 processor only while it processes audio.
    serum([], duration=128 / 48000, sample_rate=48000, num_channels=2, reset=False)
    xml = serum.raw_state[8:].decode()
    _, _, cbor = container_read(juce_b64_decode(re.search(r"<IComponent>(.*?)</IComponent>", xml).group(1)))
    return flat(cbor_decode(cbor)[0], "", {})

def main():
    serum = load_plugin(SERUM, plugin_name="Serum 2")
    base = snapshot(serum)
    points = [0.0, 0.25, 0.5, 0.75, 1.0]
    result, t = [], time.time()
    for idx, (name, p) in enumerate(serum.parameters.items()):
        default = p.raw_value
        entry = {"name": name, "default_raw": default, "default_text": p.string_value, "field": None, "plain": []}
        fields = set()
        base = snapshot(serum)  # fresh, so earlier parameters' restores don't leak in
        for x in points:
            p.raw_value = x
            snap = snapshot(serum)
            changed = {k: v for k, v in snap.items() if base.get(k) != v}
            fields |= set(changed)
            entry["plain"].append([x, p.string_value, changed])
        p.raw_value = default
        entry["fields"] = sorted(fields)
        result.append(entry)
        if idx % 250 == 0:
            print(f"{idx} params, {time.time() - t:.0f}s", flush=True)
    json.dump(result, open("serum_param_map.json", "w"))
    print(f"done: {len(result)} params in {time.time() - t:.0f}s")


if __name__ == "__main__":
    main()
