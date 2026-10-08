"""Ganze Region in einem Bild: links Block 1 (Hauptkarte), rechts die Tail-Pyramide."""
from PIL import Image
import sys
OUT='/home/marius/git/teasi/terrain_tiles'
for (x,y) in [(134,25),(136,22)]:
    b1=Image.open(f'{OUT}/r_{x}_{y}_block1.jpg')
    pyr=Image.open(f'{OUT}/r_{x}_{y}_pyr.jpg')
    sc=1024/b1.size[1]
    b1s=b1.resize((int(b1.size[0]*sc),1024),Image.LANCZOS)
    psc=1024/pyr.size[1]
    pyrs=pyr.resize((int(pyr.size[0]*psc),1024),Image.LANCZOS)
    out=Image.new('RGB',(b1s.size[0]+20+pyrs.size[0],1024),(20,20,20))
    out.paste(b1s,(0,0)); out.paste(pyrs,(b1s.size[0]+20,0))
    out.save(f'{OUT}/region_full_{x}_{y}.jpg',quality=92)
    nt=b1.size[1]//256
    print(f"({x},{y}): Block1 {b1.size} ({nt} Zeilen) + Pyramide {pyr.size} -> {out.size}")
