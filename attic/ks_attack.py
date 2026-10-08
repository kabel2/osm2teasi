"""Known-Plaintext-Angriff auf die Falko-Chart-Payloads (osmarea/osmpoint).

Modell (aus OSMAAREA_FORMAT.md):
    ct = pt XOR K_g   mit K_g = f(blob_g), Keystream-Reset pro Record, kein IV.
Angriff:
    1) Records mit identischem Ciphertext-Präfix clustern (gleiche Gruppe).
    2) XOR innerhalb des Clusters -> Positionen mit XOR==0 in ALLEN Paaren sind
       "stabil". Wenn dort das Klartext-Byte 0 ist (Padding), dann K[i]==ct[i].
    3) KDF-Suche: vergleicht (partielle) Keystreams gegen Kandidaten aus
       blob / 64-B-Headerblock / (x,y).
"""
import struct, hashlib, sys, collections

BASE='/home/marius/git/teasi/2013021200000368/7/943/20317/'

def parse(name, nrec):
    d=open(BASE+name,'rb').read()
    magic=struct.unpack_from('<I',d,0)[0]
    fblk=d[4:68]
    n=struct.unpack_from('<I',d,0x74)[0]
    dirs=[]
    for i in range(n):
        o=0x78+8*i
        w,off=struct.unpack_from('<II',d,o)
        x,y=w&0xFFFF,w>>16
        dirs.append((x,y,off))
    regs=[]
    for i,(x,y,off) in enumerate(dirs):
        end=dirs[i+1][2] if i+1<n else len(d)
        blob=d[off+0x157C:off+0x159C]
        z=off+0x159C
        recs=[]
        p=z
        while True:
            if nrec>0 and len(recs)>=nrec: break
            if p+8>end: break
            ln,pltx=struct.unpack_from('<II',d,p)
            if ln<1 or p+8+ln>end: break
            recs.append((p-z,ln,pltx,bytes(d[p+8:p+8+ln])))
            p+=8+ln
        regs.append(dict(x=x,y=y,off=off,blob=blob,recs=recs,full=(p==end)))
    return d,fblk,regs

def cluster_key(b,n=8): return bytes(b[:n])

def analyze(name, nrec, label):
    d,fblk,regs=parse(name,nrec)
    print(f'### {label}: {name} magic={d[0]:04x} regs={len(regs)}')
    groups=collections.defaultdict(list)   # blob -> list of (x,y,ct)
    for r in regs:
        if r['blob']==b'\xff'*32: continue
        for (off,ln,pltx,ct) in r['recs']:
            groups[r['blob']].append((r['x'],r['y'],off,ln,pltx,ct))
    print(f'    groups={len(groups)} records={sum(len(v) for v in groups.values())}')
    kdf_hits=[]
    for gi,(blob,recs) in enumerate(sorted(groups.items(), key=lambda kv: min(c[0] for c in kv[1]))):
        types=collections.Counter(c[5][0] for c in recs)
        typ=types.most_common(1)[0][0]
        K0=[c[5][0]^typ for c in recs]
        cons = len(set(K0))==1
        clusters=collections.defaultdict(list)
        for c in recs: clusters[cluster_key(c[5])].append(c)
        cl_sizes=sorted((len(v) for v in clusters.values()),reverse=True)
        line=f'    g{gi:02d} ({recs[0][0]},{recs[0][1]}) n={len(recs):4d} typ={typ:02x} K0={"const" if cons else "VARY"} K0[0]={K0[0]:02x} clusters={cl_sizes[:8]}'
        print(line)
        # --- Clusters mit >=2 Records: XOR-Analyse ---
        for key,cl in clusters.items():
            if len(cl)<2: continue
            L=min(len(c[5]) for c in cl)
            # stabile Bytes: identisch in allen Records
            stable=[]
            for i in range(L):
                if len(set(c[5][i] for c in cl))==1:
                    stable.append(i)
            # XOR-Distanz aller Paare
            import itertools
            dist=[sum(1 for a,b in zip(x[5][:L],y[5][:L]) if a!=b) for x,y in itertools.combinations(cl,2)]
            mdist=max(dist)
            # Kandidat: stabile Bytes -> wenn PT dort 0, dann K[i]=ct[i]
            K_partial=bytes(c[5][i] for i in stable)
            kdf_hits.append((gi,recs[0][0],recs[0][1],len(cl),L,mdist,len(stable),blob,K_partial, cl[0][5][:16]))
            if len(cl)>=3 or mdist<8:
                print(f'       cl n={len(cl)} L={L} maxdist={mdist} stable={len(stable)} ct0={cl[0][5][:12].hex()}')
                for i in range(min(6,len(stable))):
                    print(f'         stable[{stable[i]}]={cl[0][5][stable[i]]:02x}')
    # --- KDF-Suche auf stabile Bytes ---
    print('    --- KDF-Tests (stabile Bytes vs. Kandidaten) ---')
    seen=set()
    for (gi,x,y,cln,L,mdist,nstab,blob,K_partial,ct16) in kdf_hits:
        if nstab<8 or cln<2: continue
        key=(gi,x,y)
        if key in seen: continue
        seen.add(key)
        cands={
          'blob[0]':blob[0],'blob[15]':blob[15],'blob[16]':blob[16],'blob[31]':blob[31],
          'blobxor':__import__('functools').reduce(lambda a,b:a^b,blob),
          'sum%256':sum(blob)%256,
          'md5(blob)[0]':hashlib.md5(blob).digest()[0],
          'sha1(blob)[0]':hashlib.sha1(blob).digest()[0],
          'sha256(blob)[0]':hashlib.sha256(blob).digest()[0],
          'md5(fb)[0]':hashlib.md5(fblk).digest()[0],
          'md5(fb+blob)[0]':hashlib.md5(fblk+blob).digest()[0],
          'md5(blob+fb)[0]':hashlib.md5(blob+fblk).digest()[0],
          'md5(fb+xy)<I0':hashlib.md5(fblk+struct.pack('<2H',x,y)).digest()[0],
          'md5(x+fb+y)':hashlib.md5(struct.pack('<H',x)+fblk+struct.pack('<H',y)).digest()[0],
          'md5(blob+xy)':hashlib.md5(blob+struct.pack('<2H',x,y)).digest()[0],
        }
        hit=[k for k,v in cands.items() if v==K_partial[0]]
        if hit:
            print(f'       *** HIT g{gi} ({x},{y}): {hit} == {K_partial[0]:02x}')
    print()

if __name__=='__main__':
    if len(sys.argv)>1 and sys.argv[1]=='point':
        analyze('Denmark_osmpoint.v20210916',0,'OSMPOINT')
    else:
        analyze('Denmark_osmarea.v20210810',16,'OSMAREA')
