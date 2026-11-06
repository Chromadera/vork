"""Measure reprojection and hand clearance. Evidence stays in the media directory.

Held-out desk-prop tracks are independent of the camera fit. Glass reflections
are NOT stationary table features. Contact predictions are compared with a
second solve from held-out tracks: this is model disagreement, not ground truth.
"""
import argparse
import json
from pathlib import Path
import cv2
import numpy as np
from scipy.optimize import least_squares
from track import project, timestamps, FOCAL
from scipy.spatial.transform import Rotation


def stats(values):
    values = np.asarray(values)
    return dict(mean_px=float(values.mean()), max_px=float(values.max()))


def verify(media):
    cv2.setNumThreads(4)
    data = np.load(media/'lock-tracks.npz')
    tracks, world, poses = (data[k] for k in ['tracks', 'world', 'poses'])
    fit, held = data['fit_indices'], data['validation']
    scene = json.loads((media/'clean.json').read_text())
    authored = {tuple(t['path']): t['keys'] for layer in scene['animation'] for t in layer['tracks']}
    for path in [('nodes','camera','translation'), ('nodes','camera','rotation'), ('materials','camera_optics','inputs','lens')]:
        assert np.allclose([k[0] for k in authored[path]], np.arange(len(poses))/25, atol=1e-9), 'camera key times differ from plate clock'
    translation = np.array([k[1] for k in authored[('nodes','camera','translation')]])
    rotation = Rotation.from_quat([k[1] for k in authored[('nodes','camera','rotation')]]).as_matrix()
    assert np.allclose(translation, poses[:,3:], atol=1e-6), 'scene camera differs from measured camera'
    assert np.allclose(rotation, Rotation.from_rotvec(poses[:,:3]).as_matrix(), atol=1e-6)
    assert all(abs(k[1]-FOCAL*36/1920)<1e-6 for k in authored[('materials','camera_optics','inputs','lens')])
    assert scene['render']['resolution']==[1920,1080] and scene['render']['fps']==25
    assert scene['render']['motion_blur']['shutter']==0
    assert all(scene['images'][name].get('time') is None for name in ['plate','person','hand']), 'unexpected image.time override'
    layout = json.loads((media/'proxy-layout.json').read_text())
    contacts = np.array(layout['contact_points'])
    projected = np.array([project(p, world) for p in poses])
    residual = projected-tracks
    contact_uv = np.array([project(p, contacts) for p in poses])
    edge_world, edge_observed, edge_slope = (data[k] for k in ['edge_world','edge_observed','edge_slope'])
    edge_projected = np.array([project(p, edge_world) for p in poses])
    edge_residual = (edge_projected[:,:,1]-edge_observed[:,:,1]-edge_slope[:,None]*(edge_projected[:,:,0]-edge_observed[:,:,0])) / np.sqrt(1+edge_slope[:,None]**2)
    independent = []
    for i, (p, xy) in enumerate(zip(poses, tracks)):
        def independent_residual(v):
            edge = project(v, edge_world[1:2])[0]
            normal = edge[1]-edge_observed[i,1,1]-edge_slope[i]*(edge[0]-edge_observed[i,1,0])
            return np.r_[(project(v,world[held])-xy[held]).ravel(), 5*normal]
        check = least_squares(independent_residual,
                              p, loss='soft_l1', f_scale=.5, max_nfev=100)
        independent.append(project(check.x, contacts))
    disagreement = np.linalg.norm(np.array(independent)-contact_uv, axis=2)
    report = dict(camera_fit=stats(np.linalg.norm(residual[:, fit], axis=2)),
                  held_out_features=stats(np.linalg.norm(residual[:, held], axis=2)),
                  held_out_interframe=stats(np.linalg.norm(np.diff(residual[:, held], axis=0), axis=2)),
                  contact_model_disagreement=stats(disagreement),
                  held_out_table_edge_normal=stats(np.abs(edge_residual[:,[1,4]])),
                  held_out_left_edge_normal=stats(np.abs(edge_residual[:,1])),
                  held_out_central_edge_normal=stats(np.abs(edge_residual[:,4])),
                  held_out_table_edge_second_half=stats(np.abs(edge_residual[100:,[1,4]])),
                  table_lip_height_m=float(data['edge_height']),
                  fit_tracks=len(fit), held_out_tracks=len(held), frames=len(poses),
                  subpixel_tabletop_lock_proven=False,
                  limitations=['The table edge measures normal slip only; glass reflections cannot measure tangential contact slip.',
                    'Contact model disagreement is an uncertainty estimate, not measured ground-truth slip.',
                    'Focal length, table height and scale are estimated; lens distortion and rolling shutter are unsolved.'])
    old = media/'original-camera-solve.json'
    if old.exists():
        old = json.loads(old.read_text())
        prediction = np.array([tracks[0]@np.array(r['matrix'])[:, :2].T+np.array(r['matrix'])[:, 2] for r in old])
        report['original_fit_on_same_held_out_features'] = stats(np.linalg.norm(prediction[:, held]-tracks[:, held], axis=2))
    report['timing'] = {}
    for name in ['plate.mp4', 'person.mkv', 'hand.mkv']:
        ts = timestamps(media/name)
        report['timing'][name] = dict(frames=len(ts), max_error_s=float(np.max(np.abs(ts-np.arange(len(ts))/25))))
    # Conservative projected hull of all artifact vertices against the tracked hand.
    seed = next(n for n in scene['nodes'] if n['name']=='seed')
    assert np.allclose(seed['scale'], layout['seed_scale'])
    assert np.allclose(seed['translation'], np.array(layout['artifact'])*(1-layout['seed_scale']))
    vertices = []
    for node in seed['children']:
        name = scene['geometry'][node['geometry']]['attributes']['POSITION']
        buffer = scene['buffers'][name]
        points = np.fromfile(media/buffer['file'], dtype='<f4', count=buffer['count']*3,
                             offset=buffer['offset']).reshape(-1, 3)
        vertices.extend(points*np.array(seed['scale'])+np.array(seed['translation']))
    vertices = np.array(vertices)
    clearance, overlaps, rows = [], [], []
    out = media/'lock-overlays'
    out.mkdir(exist_ok=True)
    cap = cv2.VideoCapture(str(media/'plate.mp4'))
    for i, pose in enumerate(poses):
        ok, frame = cap.read()
        if not ok: raise RuntimeError(f'plate missing frame {i}')
        hull = cv2.convexHull(project(pose, vertices).astype('float32')).round().astype('int32')
        seed_mask = np.zeros(frame.shape[:2], 'uint8')
        cv2.fillConvexPoly(seed_mask, hull, 1)
        hand = cv2.imread(str(media/'hand-mattes'/f'{i:06d}.png'), cv2.IMREAD_UNCHANGED)>32767
        distance = cv2.distanceTransform((~hand).astype('uint8'), cv2.DIST_L2, cv2.DIST_MASK_PRECISE)
        clearance.append(float(distance[seed_mask>0].min()))
        overlaps.append(int(np.count_nonzero(hand & (seed_mask>0))))
        cv2.polylines(frame, [hull], True, (0,255,255), 1)
        for xy in contact_uv[i]: cv2.drawMarker(frame, tuple(np.rint(xy).astype(int)), (255,255,0), cv2.MARKER_CROSS, 9, 1)
        for xy in edge_projected[i]: cv2.drawMarker(frame, tuple(np.rint(xy).astype(int)), (255,0,255), cv2.MARKER_CROSS, 9, 1)
        for j in held:
            a, b = tuple(np.rint(tracks[i,j]).astype(int)), tuple(np.rint(projected[i,j]).astype(int))
            cv2.circle(frame, a, 3, (0,255,0), 1)
            cv2.drawMarker(frame, b, (0,0,255), cv2.MARKER_CROSS, 7, 1)
        cv2.putText(frame, f'{i:03d} hand clearance {clearance[-1]:.1f}px', (700,1045), 0,.7,(255,255,255),2)
        cv2.imwrite(str(out/f'{i:06d}.jpg'), frame[650:1080,600:1500])
        rows.append(dict(frame=i, time=i/25, contact_uv=contact_uv[i].tolist(),
                         held_out_observed=tracks[i,held].tolist(), held_out_projected=projected[i,held].tolist(),
                         table_edge_observed=edge_observed[i].tolist(), table_edge_projected=edge_projected[i].tolist(),
                         table_edge_normal_error_px=edge_residual[i].tolist(),
                         contact_model_disagreement_px=disagreement[i].tolist(), hand_clearance_px=clearance[-1],
                         hand_overlap_pixels=overlaps[-1],artifact_bounds_px=[hull[:,0,:].min(axis=0).tolist(),hull[:,0,:].max(axis=0).tolist()]))
    cap.release()
    report['artifact_size_px'] = dict(min_height=min(r['artifact_bounds_px'][1][1]-r['artifact_bounds_px'][0][1] for r in rows), max_height=max(r['artifact_bounds_px'][1][1]-r['artifact_bounds_px'][0][1] for r in rows))
    report['hand'] = dict(min_clearance_px=min(clearance), max_overlap_pixels=max(overlaps),
                          closest_frame=int(np.argmin(clearance)), method='conservative projected convex hull vs SAM hand at 50% coverage')
    (media/'lock-projections.json').write_text(json.dumps(rows, indent=2)+'\n')
    (media/'lock-report.json').write_text(json.dumps(report, indent=2)+'\n')
    print(json.dumps(report, indent=2))


if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('media',type=Path)
    verify(p.parse_args().media.resolve())
