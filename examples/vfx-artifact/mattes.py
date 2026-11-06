"""External plate tool: SAM 2.1 video masks; run with torch and upstream sam2.

The model and all results belong outside the checkout. No runtime dependency
on this tool is added to vork. SAM 2 code/checkpoints: Apache 2.0.
"""
import argparse
from pathlib import Path
import subprocess


def run(media, checkpoint, hand=False):
    import cv2
    import numpy as np
    import torch
    from sam2.build_sam import build_sam2_video_predictor

    frames = media/'sam-frames'
    frames.mkdir(exist_ok=True)
    subprocess.run(['ffmpeg', '-v', 'error', '-y', '-i', str(media/'plate.mp4'),
                    '-q:v', '2', str(frames/'%06d.jpg')], check=True)
    predictor = build_sam2_video_predictor('configs/sam2.1/sam2.1_hiera_s.yaml',
                                          str(checkpoint), device='cuda')
    out = media/('hand-mattes' if hand else 'mattes')
    out.mkdir(exist_ok=True)
    with torch.inference_mode(), torch.autocast('cuda', dtype=torch.bfloat16):
        state = predictor.init_state(str(frames), offload_video_to_cpu=True)
        if hand:
            predictor.add_new_points_or_box(state, frame_idx=0, obj_id=1,
                box=np.array([830,680,1275,860],np.float32),
                points=np.array([[1080,748],[1205,778],[950,790],[1120,730],
                                 [1120,640],[1260,690],[1100,870],[1350,800]],np.float32),
                labels=np.array([1,1,1,1,0,0,0,0],np.int32))
        else:
            # Positive points on the sleeve, face, wrist and hair; negatives on
            # the laptop/table keep the arm's soft silhouette separate from props.
            predictor.add_new_points_or_box(state, frame_idx=0, obj_id=1,
                box=np.array([180,320,1420,975],np.float32),
                points=np.array([[650,720],[1125,610],[960,775],[1200,465],
                                 [940,550],[700,600],[400,850],[800,650],
                                 [1040,865],[800,940],[1420,550],[800,400]], np.float32),
                labels=np.array([1,1,1,1,1,1,1,1,0,0,0,0], np.int32))
        for frame, ids, logits in predictor.propagate_in_video(state):
            # SAM logits are segmentation confidence, not optical opacity.
            # Convert their boundary to a subpixel-feathered coverage matte.
            mask=(logits[0,0]>0).float().cpu().numpy()
            mask=cv2.morphologyEx(mask,cv2.MORPH_CLOSE,np.ones((3,3),np.uint8))
            # The subject is one connected silhouette. Remove the occasional
            # detached chair/floor strip propagated by the video predictor.
            count,components,stats,_=cv2.connectedComponentsWithStats(np.uint8(mask),8)
            if count>1:
                subject=1+np.argmax(stats[1:,cv2.CC_STAT_AREA])
                mask=np.float32(components==subject)
            contours,hierarchy=cv2.findContours(np.uint8(mask),cv2.RETR_CCOMP,cv2.CHAIN_APPROX_SIMPLE)
            if hierarchy is not None:
                for contour,relation in zip(contours,hierarchy[0]):
                    if relation[3]>=0 and cv2.contourArea(contour)<512:
                        cv2.drawContours(mask,[contour],-1,1.,cv2.FILLED)
            alpha=cv2.GaussianBlur(mask,(0,0),.55)
            cv2.imwrite(str(out/f'{frame:06d}.png'), np.uint16(alpha*65535))
    subprocess.run(['ffmpeg','-v','error','-y','-framerate','25','-i',str(out/'%06d.png'),
                    '-c:v','ffv1','-pix_fmt','gray16le',str(media/('hand.mkv' if hand else 'person.mkv'))], check=True)


if __name__ == '__main__':
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('media',type=Path)
    p.add_argument('checkpoint',type=Path)
    p.add_argument('--hand',action='store_true')
    a=p.parse_args()
    run(a.media.resolve(), a.checkpoint.resolve(), a.hand)
