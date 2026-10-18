import sys, numpy as np
from harmonics import read
for name in sys.argv[2:]:
    r, sr = read(f"{sys.argv[1]}/reference/{name}.wav"); l, _ = read(f"{sys.argv[1]}/lumen/{name}.wav")
    seg = lambda x: x[int(0.4*sr):int(1.4*sr)]
    db = lambda v: 20*np.log10(np.sqrt((v**2).mean())+1e-12)
    print(f"{name:24} serum L {db(seg(r)[:,0]):6.1f} R {db(seg(r)[:,1]):6.1f} | lumen L {db(seg(l)[:,0]):6.1f} R {db(seg(l)[:,1]):6.1f}")
