#!/usr/bin/env python3
"""Check local listing limits, branding and PNG geometry without dependencies."""
from pathlib import Path
import struct
root=Path(__file__).resolve().parents[1]
meta=root/'fastlane/metadata/android/en-US'
for name,limit in [('title.txt',30),('short_description.txt',80),('full_description.txt',4000),('changelogs/1.txt',500)]:
    value=(meta/name).read_text().strip()
    assert 0<len(value)<=limit,(name,len(value),limit)
    assert 'TODO' not in value and 'PLACEHOLDER' not in value,name
    print(f'{name}: {len(value)}/{limit} characters')
for name,size,color in [('icon.png',(512,512),6),('featureGraphic.png',(1024,500),2)]:
    data=(meta/'images'/name).read_bytes()
    assert data[:8]==b'\x89PNG\r\n\x1a\n'
    width,height,depth,kind=struct.unpack('>IIBB',data[16:26])
    assert (width,height)==size and depth==8 and kind==color,(name,width,height,depth,kind)
    if name=='icon.png': assert len(data)<=1024*1024
    print(f'{name}: {width}x{height}')
screens=list((meta/'images/phoneScreenshots').glob('*.png'))
assert 2<=len(screens)<=8,'Need 2–8 real phone screenshots'
for path in screens:
    data=path.read_bytes();w,h,depth,kind=struct.unpack('>IIBB',data[16:26])
    assert min(w,h)>=320 and max(w,h)<=3840 and max(w,h)<=2*min(w,h),path
    assert depth==8 and kind==2,(path,'Screenshot must be 24-bit RGB')
assert (root/'PRIVACY.md').read_text()==(root/'app/src/main/assets/privacy.txt').read_text(),'Bundled privacy policy must match'
print(f'{len(screens)} valid phone screenshots; privacy policy matches app asset.')
