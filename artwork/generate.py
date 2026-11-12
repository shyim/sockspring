#!/usr/bin/env python3
"""Render original vector-style store artwork. Requires Pillow and a TrueType font."""
from pathlib import Path
import argparse
from PIL import Image, ImageDraw, ImageFont
parser=argparse.ArgumentParser()
parser.add_argument('--font',required=True,help='Path to a sans-serif TrueType font')
args=parser.parse_args()
root=Path(__file__).resolve().parents[1]
out=root/'fastlane/metadata/android/en-US/images'
out.mkdir(parents=True,exist_ok=True)
lime='#D9F29E';ink='#202C0D';cream='#FAF9F6';purple='#DCD5F2'
scale=3

def icon(draw,x,y,size):
    # Same geometry as Android launcher_foreground.xml, expanded for the store icon.
    k=size/64
    for a,b,w,h in [(12,8,14,14),(38,8,14,14),(12,42,14,14),(38,42,14,14),(18,22,4,20),(42,22,4,20),(26,13,12,4),(26,47,12,4)]:
        draw.rectangle((int((x+a*k)*scale),int((y+b*k)*scale),int((x+(a+w)*k)*scale),int((y+(b+h)*k)*scale)),fill=ink)

def font(size):return ImageFont.truetype(args.font,size*scale)
def text(draw,pos,value,size,fill=ink):draw.text(tuple(int(v*scale) for v in pos),value,font=font(size),fill=fill)
image=Image.new('RGBA',(512*scale,512*scale),lime);draw=ImageDraw.Draw(image)
icon(draw,64,64,384)
image.resize((512,512),Image.Resampling.LANCZOS).save(out/'icon.png')
image=Image.new('RGB',(1024*scale,500*scale),cream);draw=ImageDraw.Draw(image)
draw.rounded_rectangle((680*scale,68*scale,958*scale,432*scale),radius=95*scale,fill=lime)
draw.ellipse((854*scale,17*scale,981*scale,144*scale),fill=purple)
icon(draw,707,141,224)
text(draw,(64,115),'Sockspring',66)
text(draw,(68,214),'Your phone. Your proxy.',29)
text(draw,(68,328),'SOCKS5 for Android',21)
image.resize((1024,500),Image.Resampling.LANCZOS).save(out/'featureGraphic.png')
print('Generated icon.png (512x512 RGBA) and featureGraphic.png (1024x500 RGB).')
