"""Tail = Mipmap-Pyramide derselben Fläche: 8x8 (Block 1) -> 4x4 -> 2x2 -> 1x1.
Regel: pro Ebene feste Zellenzahl (16, 4, 1), zeilenweise, Zeile 0 = Süd,
'(-3,0)'-Marker = leere Zelle. Verifiziert per Template-Match gegen Block 1."""
import struct, json, io
from PIL import Image
PATH='/home/marius/git/teasi/2013021200000368/7/943/20317/Denmark_terrain.v20210916'
OUT='/home/marius/git/teasi/terrain_tiles'
d=open(PATH,'rb').read()
JS=json.load(open('/tmp/opencode/tile_index.json'))['regions']
def seq(reg):
    rs=reg['rec_off']; first=min(reg['jpeg'])
    st=[struct.unpack_from('<I',d,rs+4*j)[0] for j in range((first-rs)//4)]
    out=[];exp=None;j=0;started=False
    while j+1<len(st):
        a,b=st[j],st[j+1]
        if a==0xFFFFFFFF:
            if started: break
            j+=2;continue
        started=True
        if a==0xFFFFFFFD and b==0: out.append('S');j+=2;continue
        if exp is None or a==exp: out.append('T');exp=a+b;j+=2
        else: j+=1
    if out and out[-1]=='T': out=out[:-1]
    return out
def tile(reg,i,tp=256):
    o=reg['jpeg'][i]
    im=Image.open(io.BytesIO(d[o:o+80000]));im.load()
    return im.convert('RGB').rotate(90,expand=True).resize((tp,tp),Image.LANCZOS)
def build(x,y):
    reg=[r for r in JS if r['x']==x and r['y']==y][0]
    s=seq(reg); n1=len(reg['jp2']); nt=len(reg['jpeg'])
    ti=0;used=0;maxr=0;r=c=0
    for e in s:
        if ti>=n1: break
        used+=1
        if e=='T': ti+=1;maxr=max(maxr,r)
        c+=1
        if c==8:
            c=0;r+=1
    rows1=maxr+1
    rest=s[used:]
    H1=(rows1+1)//2; H2=(H1+1)//2
    levels=[(4,H1),(2,H2),(1,1)]
    pos=0;panels=[]
    for (W,H) in levels:
        img=Image.new('RGB',(W*256,H*256),(140,140,140));filled=0;k=0
        for rr in range(H):
            for cc in range(W):
                if pos>=len(rest): break
                e=rest[pos];pos+=1
                if e=='T':
                    if ti<nt:
                        img.paste(tile(reg,ti),(cc*256,(H-1-rr)*256));ti+=1;filled+=1
                k+=1
        panels.append((f'{W}x{H}',img,filled))
    Htot=max(p[1].size[1] for p in panels)
    out=Image.new('RGB',(sum(p[1].size[0] for p in panels)+20*(len(panels)-1),Htot),(30,30,30))
    ox=0
    for nm,img,f in panels:
        out.paste(img,(ox,Htot-img.size[1]));ox+=img.size[0]+20
    out.save(f'{OUT}/pyr2_{x}_{y}.jpg',quality=92)
    left=len(rest)-pos
    return rows1,[f'{nm}:{f}' for nm,img,f in panels],left,rest[pos:].count('T')
print(f"{'Region':>10} {'B1-Zeilen':>9}  Ebenen               Rest(Zellen/Tiles)")
for reg in JS:
    if not reg['jpeg']: continue
    r1,lv,left,lt=build(reg['x'],reg['y'])
    print(f"({reg['x']:3},{reg['y']:3}) {r1:>9}  {' '.join(f'{v:<9}' for v in lv)}  {left}/{lt}")
