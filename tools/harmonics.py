"""Harmonic levels (dB re fundamental) and RMS of reference vs Lumen renders for selected probes."""
import sys, wave, struct, json, numpy as np
def read(path):
    with open(path, "rb") as f: d = f.read()
    i = 12; fmt = data = None
    while i + 8 <= len(d):
        cid, n = d[i:i+4], struct.unpack_from("<I", d, i+4)[0]
        if cid == b"fmt ": fmt = struct.unpack_from("<HHIIHH", d, i+8)
        if cid == b"data": data = d[i+8:i+8+n]
        i += 8 + n + (n & 1)
    x = np.frombuffer(data, dtype="<f4").reshape(-1, fmt[1])
    return x, fmt[2]
def harm(x, sr, f0, k=12):
    seg = x[int(0.4*sr):int(1.4*sr)].mean(axis=1)
    w = np.hanning(len(seg)); sp = np.abs(np.fft.rfft(seg*w)); fr = np.fft.rfftfreq(len(seg), 1/sr)
    amps = []
    for h in range(1, k+1):
        b = np.argmin(abs(fr - h*f0)); amps.append(sp[max(b-3,0):b+4].max())
    a = np.array(amps); return 20*np.log10(a/a[0] + 1e-12), 20*np.log10(np.sqrt((seg**2).mean())+1e-12)
def main():
    run = sys.argv[1]
    f0 = 261.6256
    for name in sys.argv[2:]:
        r, sr = read(f"{run}/reference/{name}.wav"); l, _ = read(f"{run}/lumen/{name}.wav")
        hr, rr = harm(r, sr, f0); hl, rl = harm(l, sr, f0)
        print(f"{name}: rms serum {rr:6.1f} lumen {rl:6.1f} dB | peak serum {np.abs(r).max():.3f} lumen {np.abs(l).max():.3f}")
        print("   serum H1..12:", " ".join(f"{v:6.1f}" for v in hr))
        print("   lumen H1..12:", " ".join(f"{v:6.1f}" for v in hl))

if __name__ == "__main__":
    main()
