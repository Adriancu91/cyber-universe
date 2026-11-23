#!/usr/bin/env python3
"""P6-153: checks the OS's batch windows against PyTorch and Kronos's own `module/date_bucati.py` (read, not copied).
  python3 tools/date-bucati-lot.py <cyberspace-dir> [<data-dir>]
Prints (1) MT19937 = torch.randint on several seeds and ranges (both the 32-bit and the 64-bit path), (2) the lots
the `antrenare-lot` scenario expects, computed by Kronos's `DateBucati.lot`. Needs torch (CPU is enough)."""
import sys, hashlib, tempfile, torch
sys.path.insert(0, sys.argv[1])
from module.date_bucati import DateBucati
class MT:
    def __init__(s, seed):
        st=[seed & 0xffffffff]
        for j in range(1,624): st.append((1812433253*(st[-1]^(st[-1]>>30))+j)&0xffffffff)
        s.st=st; s.i=624
    def u32(s):
        if s.i>=624:
            st=s.st
            for i in range(624):
                y=(st[i]&0x80000000)|(st[(i+1)%624]&0x7fffffff)
                v=st[(i+397)%624]^(y>>1)
                if y&1: v^=0x9908b0df
                st[i]=v
            s.i=0
        y=s.st[s.i]; s.i+=1
        y^=y>>11; y^=(y<<7)&0x9d2c5680; y^=(y<<15)&0xefc60000; return (y^(y>>18))&0xffffffff
    def randint(s,r):
        if r>=1<<28:
            hi=s.u32(); lo=s.u32(); return ((hi<<32)|lo)%r
        return s.u32()%r
# 1) MT vs torch on many seeds/ranges incl. >= 2^32
for seed in [0,1,24,5489,24*1000003+7, 2**40+3]:
    for r in [5, 2633, 2**28-1, 2**28, 2**31+11, 2**32+5, 2700000000, 2**40+17]:
        g=torch.Generator().manual_seed(seed); t=torch.randint(r,(700,),generator=g).tolist()
        m=MT(seed); mine=[m.randint(r) for _ in range(700)]
        assert t==mine,(seed,r,t[:3],mine[:3])
print("MT19937 = torch.randint: OK (6 seminte x 8 intervale x 700)")
# 2) Kronos lots on the test data
d=sys.argv[2] if len(sys.argv) > 2 else "tests/disk/antrenare-date/antrenare"
val=tempfile.NamedTemporaryFile("w",suffix=".txt",delete=False); val.write("x"*200); val.close()
db=DateBucati(d+"/date", val.name, d+"/tokenizer.json")
def run(samanta, pasi, loturi, numar, ctx):
    seed = samanta if pasi==0 else samanta*1000003+pasi
    g=torch.Generator(device="cpu").manual_seed(seed)
    print(f"samanta {samanta} pasi_total {pasi} -> {seed}")
    for l in range(loturi):
        gg=g.get_state()
        pos=torch.randint(len(db._sir)-ctx-1,(numar,),generator=g).tolist()
        g.set_state(gg)
        x,y=db.lot(numar,ctx,g)
        h=hashlib.sha256(x.numpy().astype("<i8").tobytes()+y.numpy().astype("<i8").tobytes()).hexdigest()[:16]
        print(f"lotul {l}: pozitii {pos[:8]}; x[0][..4] = {x[0][:4].tolist()}; amprenta {h}")
run(0,0,3,4,64)
run(24,7,1,4,512)
