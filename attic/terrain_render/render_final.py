"""Korrektes Rendern: 85 Zellen = 64 (8x8) + 16 (4x4) + 4 (2x2) + 1.
Belegung je Ebene = 2x2-Bloecke der Unterebene mit Inhalt; (-3,0) = Lücke.
Zeile 0 = Süd; Kacheln 90° CW gespeichert -> rotate(90)."""
import struct, json, io
from PIL import Image
PATH='/home/marius/git/teasi/2013021200000368/7/943/20317/Denmark_terrain.v20210916'
OUT='/home/marius/git/teasi/terrain_tiles'
d=open(PATH,'rb').read()
JS=json.load(open('/tmp/opencode/tile_index.json'))['regions']
def seq(reg):
    rs=reg['rec_off']; N=len(reg['jpeg']); out=[]; t=0
    for j in range((5532-4480)//8):
        a,b=struct.unpack_from('<II',d,rs+4480+8*j)
        if a==0xFFFFFFFF: break
        if a==0xFFFFFFFD and b==0: out.append('S'); continue
        out.append('T'); t+=1
        if t==N: break
    return out
def tile(reg,i,size):
    o=reg['jpeg'][i]
    im=Image.open(io.BytesIO(d[o:o+60000]));im.load()
    return im.convert('RGB').rotate(90,expand=True).resize((size,size),Image.LANCZOS)
def level(reg,s,a,b,W,H,tp,ti):
    """Zellen a..b in ein WxH-Raster (Zeile0=unten), Kachelgroesse tp"""
    img=Image.new('RGB',(W*tp,H*tp),(150,150,150)); n=0
    for k in range(a,b):
        if k>=len(s): break
        if s[k]=='T':
            if ti<len(reg['jpeg']):
                r,c=divmod(k-a,W)
                img.paste(tile(reg,ti,tp),(c*tp,(H-1-r)*tp)); ti+=1; n+=1
    return img,ti,n
def build(x,y):
    reg=[r for r in JS if r['x']==x and r['y']==y][0]
    s=seq(reg); N=len(reg['jpeg'])
    b1,ti,n1=level(reg,s,0,64,8,8,256,0)
    l1,ti,n1b=level(reg,s,64,80,4,4,512,ti)
    l2,ti,n2=level(reg,s,80,84,2,2,1024,ti)
    l3,ti,n3=level(reg,s,84,85,1,1,2048,ti)
    b1.save(f'{OUT}/b1_{x}_{y}.jpg',quality=92)
    W=2048*4+3*24
    out=Image.new('RGB',(W,2048),(40,40,40))
    for k,im in enumerate((b1,l1,l2,l3)): out.paste(im,(k*(2048+24),0))
    out.save(f'{OUT}/final_{x}_{y}.jpg',quality=90)
    return n1,n1b,n2,n3,ti,N
print(f"{'Region':>10}  B1  4x4  2x2  1x1  Sum  jpeg  85Zellen")
for reg in JS:
    if not reg['jpeg']: continue
    a,b,c,e,tot,N=build(reg['x'],reg['y'])
    print(f"({reg['x']:3},{reg['y']:3}) {a:>4} {b:>4} {c:>4} {e:>4} {tot:>4} {N:>5}   {len(seq(reg))}")
