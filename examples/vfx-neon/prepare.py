#!/usr/bin/env python3
"""Prepare the licensed rain plate outside the checkout."""
import argparse,hashlib,json,subprocess
from pathlib import Path
from urllib.request import urlretrieve
HERE=Path(__file__).resolve().parent

def run(media):
    media=media.resolve();checkout=HERE.parents[1]
    if media==checkout or checkout in media.parents:raise ValueError('Media must live outside the checkout')
    media.mkdir(parents=True,exist_ok=True)
    asset=json.loads((HERE/'assets.json').read_text())[0];source=media/'source.mp4'
    if not source.exists():urlretrieve(asset['url'],source)
    with source.open('rb') as stream:digest=hashlib.file_digest(stream,'sha256').hexdigest()
    if digest!=asset['sha256']:raise ValueError('Source checksum differs from assets.json')
    (media/'provenance.json').write_text(json.dumps(dict(asset,download_sha256=digest),indent=2)+'\n')
    subprocess.run(['ffmpeg','-v','error','-y','-i',str(source),'-t','5','-vf','fps=24,scale=1920:825','-c:v','libx264','-crf','10','-pix_fmt','yuv444p','-an',str(media/'track.mp4')],check=True)
    subprocess.run(['ffmpeg','-v','error','-y','-i',str(media/'track.mp4'),'-vf','pad=1920:1080:0:127','-c:v','libx264','-crf','10','-pix_fmt','yuv420p',str(media/'plate.mp4')],check=True)

if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--media',type=Path,required=True);a=p.parse_args();run(a.media)
