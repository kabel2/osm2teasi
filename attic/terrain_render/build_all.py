"""Setzt das Rezept aus CHART_FORMAT.md fuer alle 23 Regionen um:
Block 1 (8 Spalten, Zeile 0 = Sued, Lücken aus (-3,0)-Markern) + Tail-Pyramide
(16 -> 4x4, 4 -> 2x2, 1 -> Overview). Kein Kantenvergleich."""
import struct, json, io, os
from PIL import Image
PATH='/home/marius/git/teasi/2013021200000368/7/943/20317/Denmark_terrain.v20210916'
OUT='/home/marius/git/teasi/terrain_tiles'
d=open(PATH,'rb').read()
JS=json.load(open('/tmp/opencode/tile_index.json'))['regions']

def seq(reg):
    """Slot-Array AB REGION-START (der 0xFF-Block gehoert dazu: fuehrende
    Leerslots + evtl. Marker = leere Zellen am Anfang)."""
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

_cache={}
def tile(reg,i,tp):
    o=reg['jpeg'][i]
    if o not in _cache:
        im=Image.open(io.BytesIO(d[o:o+80000]));im.load()
        _cache[o]=im.convert('RGB').rotate(90,expand=True)
    im=_cache[o]
    return im.resize((tp,tp),Image.LANCZOS) if tp!=256 else im

def place(stream,tiles,n,W,tp=256):
    """legt n Tiles gemaess stream in ein W-spaltiges Raster, Zeile 0 unten."""
    g={};ti=0;r=c=0
    for e in stream:
        if ti>=n: break
        if e=='T': g[(r,c)]=ti;ti+=1
        c+=1
        if c==W: c=0;r+=1
    rows=max(k[0] for k in g)+1 if g else 0
    img=Image.new('RGB',(W*tp,rows*tp),(140,140,140))
    for (r,c),k in g.items():
        img.paste(tile(reg,k,tp),(c*tp,(rows-1-r)*tp))
    return img,rows,len(g)

def block1(reg,tp=256):     return place(seq(reg),reg['jpeg'],len(reg['jp2']),8,tp)
def grid4(reg,a,n,tp=256,W=4):
    rows=(n+W-1)//W
    img=Image.new('RGB',(W*tp,rows*tp),(140,140,140))
    for k in range(n): img.paste(tile(reg,a+k,tp),((k%W)*tp,(rows-1-k//W)*tp))
    return img

rows=[]
for reg in JS:
    x,y=reg['x'],reg['y']; n1=len(reg['jp2']); nt=len(reg['jpeg'])
    if nt==0: rows.append((x,y,0,0,0,0,'(leer)')); continue
    b1,r1,used=block1(reg,tp=256)
    b1.save(f'{OUT}/r_{x}_{y}_block1.jpg',quality=90)
    tail=nt-n1; note=''
    if tail>=16:
        fine=grid4(reg,n1,16); q=grid4(reg,n1+16,min(4,max(0,tail-16)),256,2)
        parts=[fine,q]
        if tail>=21:
            ov=Image.new('RGB',(256,256)); ov.paste(tile(reg,n1+20,256),(0,0)); parts.append(ov)
        Wp=sum(p.size[0] for p in parts)+20*(len(parts)-1); Hp=max(p.size[1] for p in parts)
        pyr=Image.new('RGB',(Wp,Hp),(30,30,30)); ox=0
        for p in parts: pyr.paste(p,(ox,0)); ox+=p.size[0]+20
        pyr.save(f'{OUT}/r_{x}_{y}_pyr.jpg',quality=90)
        note=f'Pyr 16+{min(4,tail-16)}' + ('+1' if tail>=21 else '')
    elif tail>0:
        grid4(reg,n1,tail).save(f'{OUT}/r_{x}_{y}_pyr.jpg',quality=90); note=f'Rest {tail} in 4er-Raster'
    rows.append((x,y,nt,n1,used,r1,note))

print(f"{'Region':>10} {'jpeg':>5} {'jp2':>4} {'B1':>4} {'Zeilen':>6}  Tail")
for x,y,nt,n1,used,r1,note in rows:
    print(f"({x:3},{y:3}) {nt:5} {n1:4} {used:4} {r1:6}  {note}")

# Uebersicht aller Regionen an ihren Verzeichnis-Positionen
xs=[r[0] for r in rows]; ys=[r[1] for r in rows]
W=max(xs)-min(xs)+1; H=max(ys)-min(ys)+1; CELL=256
big=Image.new('RGB',(W*CELL,H*CELL),(60,60,60))
for x,y,nt,n1,used,r1,note in rows:
    if nt==0: continue
    im=Image.open(f'{OUT}/r_{x}_{y}_block1.jpg'); im=im.resize((CELL,CELL),Image.LANCZOS)
    big.paste(im,((x-min(xs))*CELL,(max(ys)-y)*CELL))
big.save(f'{OUT}/overview_all_regions.jpg',quality=90)
print("\n-> overview_all_regions.jpg", big.size)
