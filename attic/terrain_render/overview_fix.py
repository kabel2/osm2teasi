"""Uebersicht ohne Verzerrung: Block-1-Mosaic (8 Spalten x rows Zeilen) wird
untenbuendig in eine 2048x2048-Zelle gelegt (Zeile 0 = Sued) und erst dann
verkleinert. Bisher wurde jedes Mosaic auf 256x256 gestaucht -> Verzerrung."""
import json, re, os
from PIL import Image
OUT='/home/marius/git/teasi/terrain_tiles'
JS=json.load(open('/tmp/opencode/tile_index.json'))['regions']
CELL=256
regs=[]
for f in os.listdir(OUT):
    m=re.match(r'r_(\d+)_(\d+)_block1\.jpg',f)
    if m: regs.append((int(m.group(1)),int(m.group(2)),f))
xs=[r[0] for r in regs]; ys=[r[1] for r in regs]
W=max(xs)-min(xs)+1; H=max(ys)-min(ys)+1
big=Image.new('RGB',(W*CELL,H*CELL),(60,60,60))
print(f"{'Region':>10} {'Mosaic':>12} -> Zelle (untenbuendig)")
for x,y,f in sorted(regs):
    im=Image.open(f'{OUT}/{f}')
    cell=Image.new('RGB',(2048,2048),(128,128,128))
    cell.paste(im,(0,2048-im.size[1]))          # untenbuendig: Zeile 0 = Sued
    cell=cell.resize((CELL,CELL),Image.LANCZOS)
    big.paste(cell,((x-min(xs))*CELL,(max(ys)-y)*CELL))
    print(f"({x:3},{y:3}) {im.size[0]}x{im.size[1]:>5}")
big.save(f'{OUT}/overview_fixed.jpg',quality=92)
big.resize((2304,int(2304*big.size[1]/big.size[0])),Image.LANCZOS).save(f'{OUT}/overview_fixed_preview.jpg',quality=90)
print("-> overview_fixed.jpg",big.size)
