"""chrome-qa: crop/scale/montage screenshots on the host. view.py OUT SCALE IMG[:x0,y0,x1,y1] ..."""
import sys
from PIL import Image
out, scale = sys.argv[1], float(sys.argv[2])
ims = []
for a in sys.argv[3:]:
    p, _, box = a.partition(':')
    im = Image.open(p).convert('RGB')
    if box:
        im = im.crop(tuple(map(int, box.split(','))))
    ims.append(im.resize((max(1, int(im.width * scale)), max(1, int(im.height * scale))), Image.LANCZOS if scale < 1 else Image.NEAREST))
w = sum(i.width for i in ims) + 8 * (len(ims) - 1); h = max(i.height for i in ims)
m = Image.new('RGB', (w, h), (255, 0, 255)); x = 0
for i in ims:
    m.paste(i, (x, 0)); x += i.width + 8
m.save(out)
