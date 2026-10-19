"""Render Serum 2 presets headlessly for comparison with Lumen.

usage: serum_render.py JOBS.json

JOBS.json: {"jobs": [{"preset": "C:\\...\\x.SerumPreset", "out": "C:\\...\\x.wav",
             "notes": [60], "velocity": 0.8, "hold": 1.5, "tail": 1.5}], "rate": 48000}

A preset payload is the processor state plus editor-only sections; this keeps
the processor's section list, takes each section from the preset, repackages it
as an XferJson "processor" container and loads it through the VST3 state.
"""
import hashlib, json, re, struct, sys, time, wave
import numpy as np
import zstandard
from pedalboard import load_plugin

SERUM = r"C:\Program Files\Common Files\VST3\Serum2.vst3\Contents\x86_64-win\Serum2.vst3"
ALPHA = ".ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+"


def juce_b64_decode(s):
    size, data = s.split(".", 1)
    size = int(size)
    out, bit = bytearray(size), 0
    for ch in data:
        v = ALPHA.index(ch)
        for k in range(6):
            if bit >= size * 8:
                break
            if v >> k & 1:
                out[bit >> 3] |= 1 << (bit & 7)
            bit += 1
    return bytes(out)


def juce_b64_encode(b):
    bits, out = len(b) * 8, []
    for i in range(0, bits, 6):
        v = 0
        for k in range(6):
            j = i + k
            if j < bits and b[j >> 3] >> (j & 7) & 1:
                v |= 1 << k
        out.append(ALPHA[v])
    return f"{len(b)}." + "".join(out)


# --- XferJson container (see lumen-preset for the layout) -------------------
def container_read(b):
    assert b[:9] == b"XferJson\0", "not XferJson"
    n = struct.unpack_from("<Q", b, 9)[0]
    meta = json.loads(b[17:17 + n])
    size, version = struct.unpack_from("<II", b, 17 + n)
    cbor = zstandard.ZstdDecompressor().decompress(b[17 + n + 8:], max_output_size=size)
    return meta, version, cbor


def container_write(meta, version, cbor):
    frame = zstandard.ZstdCompressor(level=3).compress(cbor)
    meta = dict(meta, hash=hashlib.md5(frame).hexdigest())
    m = json.dumps(meta, separators=(",", ":")).encode()
    return b"XferJson\0" + struct.pack("<Q", len(m)) + m + struct.pack("<II", len(cbor), version) + frame


# --- Minimal CBOR map splicing: top-level map of text keys -> raw item bytes -
def cbor_head(b, i):
    ib = b[i]; major, info = ib >> 5, ib & 31; i += 1
    if info < 24: return major, info, i
    n = {24: 1, 25: 2, 26: 4, 27: 8}[info]
    return major, int.from_bytes(b[i:i + n], "big"), i + n


def cbor_skip(b, i):
    major, arg, j = cbor_head(b, i)
    if major in (0, 1): return j
    if major in (2, 3): return j + arg
    if major == 4:
        for _ in range(arg): j = cbor_skip(b, j)
        return j
    if major == 5:
        for _ in range(2 * arg): j = cbor_skip(b, j)
        return j
    if major == 6: return cbor_skip(b, j)
    if b[i] & 31 == 25: return i + 3
    if b[i] & 31 == 26: return i + 5
    if b[i] & 31 == 27: return i + 9
    return j


def cbor_top_map(b):
    major, n, i = cbor_head(b, 0)
    assert major == 5
    entries = []
    for _ in range(n):
        ks = i; i = cbor_skip(b, i); key_raw = b[ks:i]
        _, klen, kd = cbor_head(key_raw, 0); key = key_raw[kd:kd + klen].decode()
        vs = i; i = cbor_skip(b, i)
        entries.append((key, key_raw, b[vs:i]))
    return entries


def cbor_map_bytes(entries):
    n = len(entries)
    if n < 24: head = bytes([0xA0 | n])
    elif n < 256: head = bytes([0xB8, n])
    else: head = bytes([0xB9]) + n.to_bytes(2, "big")
    return head + b"".join(k + v for _, k, v in sorted(entries, key=lambda e: e[0].encode()))


def preset_state(base_state, preset_bytes):
    """Builds a VST3 state blob with the preset's processor sections."""
    xml = base_state[8:].decode()
    comp_b64 = re.search(r"<IComponent>(.*?)</IComponent>", xml).group(1)
    meta, version, comp_cbor = container_read(juce_b64_decode(comp_b64))
    _, _, preset_cbor = container_read(preset_bytes)
    preset = {k: (kr, v) for k, kr, v in cbor_top_map(preset_cbor)}
    entries = []
    for key, key_raw, value in cbor_top_map(comp_cbor):
        kr, v = preset.get(key, (key_raw, value))
        entries.append((key, kr, v))
    blob = container_write(meta, version, cbor_map_bytes(entries))
    xml = xml.replace(comp_b64, juce_b64_encode(blob))
    body = xml.encode()
    return b"VC2!" + struct.pack("<I", len(body)) + body


def write_wav(path, audio, rate):
    data = np.ascontiguousarray(audio.T, dtype="<f4")
    with open(path, "wb") as f:
        n = data.size * 4
        f.write(b"RIFF" + struct.pack("<I", 36 + n) + b"WAVEfmt " + struct.pack("<IHHIIHH", 16, 3, 2, rate, rate * 8, 8, 32))
        f.write(b"data" + struct.pack("<I", n) + data.tobytes())


def main():
    spec = json.load(open(sys.argv[1]))
    rate = spec.get("rate", 48000)
    t = time.time()
    serum = load_plugin(SERUM, plugin_name="Serum 2")
    base = serum.raw_state
    print(f"Serum loaded in {time.time() - t:.1f}s", flush=True)
    for job in spec["jobs"]:
        t = time.time()
        try:
            serum.raw_state = preset_state(base, open(job["preset"], "rb").read())
            serum.reset()
            hold, tail, vel = job.get("hold", 1.5), job.get("tail", 1.5), int(round(job.get("velocity", 0.8) * 127))
            midi = [(bytes([0x90, n, vel]), 0.0) for n in job.get("notes", [60])]
            midi += [(bytes([0x80, n, 64]), hold) for n in job.get("notes", [60])]
            # Warm-up render lets wavetables, IRs and reverb buffers finish loading before
            # the measured one ("warmup" seconds per job; some reverb types need more).
            serum([], duration=job.get("warmup", spec.get("warmup", 0.25)), sample_rate=rate, num_channels=2)
            audio = serum(midi, duration=hold + tail, sample_rate=rate, num_channels=2, reset=False)
            write_wav(job["out"], audio, rate)
            print(f"ok {time.time() - t:5.1f}s peak {float(np.abs(audio).max()):.3f}  {job['out']}", flush=True)
        except Exception as e:  # keep going; the comparison reports missing references
            print(f"FAILED {job['preset']}: {e}", flush=True)


if __name__ == "__main__":
    main()
