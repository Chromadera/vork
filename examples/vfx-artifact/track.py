"""Track the rigid desk neighbourhood; solve a translating camera, not the seed.

The glass contains moving reflections, so use the fixed keyboard and pen/rim
features. Intrinsics and absolute scale remain estimates. Save all measurements
outside the checkout; fitting residuals are not independent validation.
"""
import json
import subprocess
from pathlib import Path
import cv2
import numpy as np
from scipy.optimize import least_squares
from scipy.ndimage import gaussian_filter1d
from scipy.spatial.transform import Rotation

FOCAL = 3200.  # 60 mm / 36 mm sensor at 1920 pixels; locally constrained estimate.
CAMERA = np.array([0., .565, 1.6])
ROTATION = Rotation.from_euler('xyz', [-.124355, 0., 0.]).as_matrix()
DECK_HEIGHT = .015


def timestamps(path):
    data = json.loads(subprocess.check_output(['ffprobe', '-v', 'error', '-select_streams', 'v:0',
        '-show_frames', '-show_entries', 'frame=best_effort_timestamp_time', '-of', 'json', str(path)]))
    times = np.array([float(f['best_effort_timestamp_time']) for f in data['frames']])
    return times-times[0]


def on_plane(pixels, height=0.):
    pixels = np.asarray(pixels).reshape(-1, 2)
    rays = np.c_[(pixels[:, 0]-960)/FOCAL, -(pixels[:, 1]-540)/FOCAL, -np.ones(len(pixels))] @ ROTATION.T
    return CAMERA+rays*((np.asarray(height).reshape(-1, 1)-CAMERA[1])/rays[:, 1, None])


def project(pose, world):
    local = (np.asarray(world)-pose[3:]) @ Rotation.from_rotvec(pose[:3]).as_matrix()
    return np.c_[960-FOCAL*local[:, 0]/local[:, 2], 540+FOCAL*local[:, 1]/local[:, 2]]


def follow(frames, polygon):
    mask = np.zeros_like(frames[0])
    cv2.fillPoly(mask, [np.array(polygon)], 255)
    p = cv2.goodFeaturesToTrack(frames[0], 300, .001, 8, mask=mask, blockSize=5)
    cv2.cornerSubPix(frames[0], p, (5, 5), (-1, -1), (3, 50, .001))
    result = [p[:, 0].copy()]
    good = np.ones(len(p), bool)
    for previous, current in zip(frames, frames[1:]):
        q, status, _ = cv2.calcOpticalFlowPyrLK(previous, current, p, None,
            winSize=(31, 31), maxLevel=4, criteria=(3, 60, .001))
        back, bs, _ = cv2.calcOpticalFlowPyrLK(current, previous, q, None,
            winSize=(31, 31), maxLevel=4, criteria=(3, 60, .001))
        good &= (status[:, 0]>0) & (bs[:, 0]>0) & (np.linalg.norm(back-p, axis=2)[:, 0]<.4)
        result.append(q[:, 0].copy())
        p = q
    return np.ascontiguousarray(np.array(result)[:, good])


def refine(frames, tracks):
    """Remove accumulated LK drift using first-frame patches, warped for perspective."""
    result = [tracks[0]]
    errors = [np.zeros(len(tracks[0]))]
    for index, gray in enumerate(frames[1:], 1):
        h, _ = cv2.findHomography(tracks[0], tracks[index], cv2.RANSAC, 1.)
        warped = cv2.warpPerspective(gray, np.linalg.inv(h), (1920, 1080))
        p = tracks[0].reshape(-1, 1, 2).copy()
        q, status, _ = cv2.calcOpticalFlowPyrLK(frames[0], warped, p, p.copy(),
            winSize=(15, 15), maxLevel=0, criteria=(3, 60, .001), flags=cv2.OPTFLOW_USE_INITIAL_FLOW)
        back, bs, _ = cv2.calcOpticalFlowPyrLK(warped, frames[0], q, p.copy(),
            winSize=(15, 15), maxLevel=0, criteria=(3, 60, .001), flags=cv2.OPTFLOW_USE_INITIAL_FLOW)
        error = np.linalg.norm(back-p, axis=2)[:, 0]
        error[(status[:, 0]==0) | (bs[:, 0]==0)] = np.inf
        result.append(cv2.perspectiveTransform(q, h)[:, 0])
        errors.append(error)
    good = np.max(errors, axis=0)<.5
    result = np.ascontiguousarray(np.array(result)[:, good])
    votes = np.zeros(result.shape[1], int)
    for xy in result:
        _, inside = cv2.findHomography(result[0], xy, cv2.RANSAC, .8)
        votes += inside[:, 0]>0
    return np.ascontiguousarray(result[:, votes>len(frames)*.85])


def desk_edge(frames, poses, tracks, world, fit_indices):
    """Constrain the insertion plane with the real front lip, not its reflection.

    Fit four spatially distributed samples and reserve two interior samples. Only the normal
    component is observable on this untextured edge. Its height is calibrated
    on the first half, then held fixed for the second half.
    """
    def locate(frame, x, y):
        x=int(round(x))
        line=frame[:, max(0,x-5):x+6].mean(axis=1)
        gradient=np.gradient(gaussian_filter1d(line,1.))
        lo,hi=max(2,int(y)-15),min(1078,int(y)+16)
        j=lo+np.argmin(gradient[lo:hi])
        curvature=gradient[j-1]-2*gradient[j]+gradient[j+1]
        return j+.5*(gradient[j-1]-gradient[j+1])/curvature
    reference=np.array([[x,locate(frames[0],x,970+(x-850)*.083)] for x in [680,740,800,850,950,1050]])
    initial=on_plane(reference)
    observed=[]
    for frame,pose in zip(frames,poses):
        observed.append([[x,locate(frame,x,y)] for x,y in project(pose,initial)])
    observed=np.array(observed)
    slope=(observed[:,-1,1]-observed[:,0,1])/(observed[:,-1,0]-observed[:,0,0])
    def height_error(height):
        predicted=np.array([project(p,on_plane(reference,height[0])) for p in poses[:100]])
        residual=predicted[:,:,1]-observed[:100,:,1]-slope[:100,None]*(predicted[:,:,0]-observed[:100,:,0])
        return residual[:,[0,2,3,5]].ravel()
    height=least_squares(height_error,[0.],loss='soft_l1',f_scale=.5).x[0]
    edge_world=on_plane(reference,height)
    for i,(pose,xy) in enumerate(zip(poses,tracks)):
        def residual(p):
            edge=project(p,edge_world[[0,2,3,5]])
            # Prioritize the insertion plane; retain rigid features for motion
            # along the edge, which a line alone cannot constrain.
            return np.r_[(project(p,world[fit_indices])-xy[fit_indices]).ravel(),
                         5*(edge[:,1]-observed[i,[0,2,3,5],1]-slope[i]*(edge[:,0]-observed[i,[0,2,3,5],0]))]
        poses[i]=least_squares(residual,pose,loss='soft_l1',f_scale=.5,max_nfev=100).x
    return dict(edge_reference=reference,edge_observed=observed,edge_world=edge_world,
                edge_slope=slope,edge_height=height)


def solve(media):
    cv2.setRNGSeed(0)
    cv2.setNumThreads(4)
    cap = cv2.VideoCapture(str(media/'plate.mp4'))
    frames = []
    while True:
        ok, frame = cap.read()
        if not ok: break
        frames.append(cv2.cvtColor(frame, cv2.COLOR_BGR2GRAY))
    cap.release()
    times = timestamps(media/'plate.mp4')
    if len(frames)!=200 or not np.allclose(times, np.arange(200)/25, atol=1e-6):
        raise ValueError('this shot requires the original 200-frame, 25 fps plate')
    deck = follow(frames, [[900,820],[1360,815],[1370,904],[880,883]])
    props = follow(frames, [[880,882],[1430,920],[1430,975],[880,970]])
    tracks = refine(frames, np.concatenate([deck, props], axis=1))
    if tracks.shape[1]<16: raise RuntimeError('insufficient full-length rigid tracks')
    # Reserve spatially distributed tracks; never use these in pose fitting.
    validation = np.argsort(tracks[0, :, 0])[::4]
    fit_indices = np.setdiff1d(np.arange(tracks.shape[1]), validation)
    world = on_plane(tracks[0], DECK_HEIGHT)
    pose = np.r_[Rotation.from_matrix(ROTATION).as_rotvec(), CAMERA]
    poses = []
    for xy in tracks:
        pose = least_squares(lambda p: (project(p, world[fit_indices])-xy[fit_indices]).ravel(),
            pose, loss='soft_l1', f_scale=.5, max_nfev=100).x
        poses.append(pose.copy())
    poses = np.array(poses)
    edge = desk_edge(frames, poses, tracks, world, fit_indices)
    rows = [dict(frame=i, time=float(times[i]), rotation=Rotation.from_rotvec(p[:3]).as_quat().tolist(),
                 translation=p[3:].tolist(), lens=FOCAL*36/1920) for i,p in enumerate(poses)]
    (media/'camera-solve.json').write_text(json.dumps(rows, indent=2)+'\n')
    np.savez(media/'lock-tracks.npz', tracks=tracks, world=world, poses=poses,
             fit_indices=fit_indices, validation=validation, props=props, **edge)
    image = cv2.cvtColor(frames[0], cv2.COLOR_GRAY2BGR)
    for i, xy in enumerate(tracks[0]):
        cv2.circle(image, tuple(np.rint(xy).astype(int)), 4,
                   (0, 180, 255) if i in validation else (0, 255, 0), 1)
    cv2.imwrite(str(media/'lock-features.jpg'), image)
    errors = np.array([np.linalg.norm(project(p, world[validation])-xy[validation], axis=1)
                       for p,xy in zip(poses,tracks)])
    print(f'{len(rows)} frames; {len(fit_indices)} fit / {len(validation)} held-out tracks; '
          f'validation mean {errors.mean():.3f}px max {errors.max():.3f}px')
    return rows


if __name__=='__main__':
    import sys
    solve(Path(sys.argv[1]))
