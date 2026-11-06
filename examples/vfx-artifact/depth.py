"""External relative-depth plate tool. Depth Anything V2 Small is Apache 2.0.

Exports inverse-depth maps for the example compositor, not a measured scan.
Model weights, intermediate arrays and movies stay outside the checkout.
"""
from pathlib import Path
import subprocess


def run(media):
    import cv2
    import numpy as np
    import torch
    from transformers import AutoImageProcessor, AutoModelForDepthEstimation
    name='depth-anything/Depth-Anything-V2-Small-hf'
    processor=AutoImageProcessor.from_pretrained(name)
    model=AutoModelForDepthEstimation.from_pretrained(name).cuda().eval()
    cap=cv2.VideoCapture(str(media/'plate.mp4'))
    out=media/'depth'
    out.mkdir(exist_ok=True)
    frame=0
    while True:
        ok,bgr=cap.read()
        if not ok: break
        inputs=processor(images=cv2.cvtColor(bgr,cv2.COLOR_BGR2RGB),return_tensors='pt').to('cuda')
        with torch.inference_mode():
            d=model(**inputs).predicted_depth
            d=torch.nn.functional.interpolate(d[:,None],size=(540,960),mode='bicubic',align_corners=False)[0,0]
        d=d.float().cpu().numpy()
        np.save(out/f'{frame:06d}.npy',d)
        frame+=1
    print(f'{frame} relative-depth frames')


if __name__=='__main__':
    import sys
    run(Path(sys.argv[1]))
