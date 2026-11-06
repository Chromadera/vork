"""Transfer Blender display settings without silently changing the authored look."""
from .view_curves import compositor_source

def translate(scene, *, on_substitution=None, emit_compositor=None):
    view = scene.view_settings
    if scene.display_settings.display_device != 'sRGB':
        raise ValueError('Unsupported display device: ' + scene.display_settings.display_device)
    if view.view_transform not in ('Standard', 'AgX', 'Filmic'):
        raise ValueError('Unsupported display transform: ' + view.view_transform)
    filmic = ('Very High Contrast', 'High Contrast', 'Medium High Contrast',
              'Medium Contrast', 'Medium Low Contrast', 'Low Contrast', 'Very Low Contrast')
    agx = {'AgX - ' + name for name in ('Punchy', 'Greyscale', 'Very High Contrast',
           'High Contrast', 'Medium High Contrast', 'Base Contrast', 'Medium Low Contrast',
           'Low Contrast', 'Very Low Contrast')}
    valid = view.look == 'None' or (view.view_transform == 'Filmic' and view.look in filmic) or (
        view.view_transform == 'AgX' and view.look in agx)
    if not valid:
        raise ValueError('Unsupported display look: ' + view.view_transform + '/' + view.look)
    inputs = dict(exposure=view.exposure, gamma=view.gamma,
                  dither_intensity=scene.render.dither_intensity)
    if view.view_transform != 'Standard':
        inputs['look'] = view.look.removeprefix('AgX - ').lower().replace(' ', '_')
    program = {'AgX': 'std/agx', 'Standard': 'std/standard', 'Filmic': 'blender/filmic'}[view.view_transform]
    result = {'display': {'program': program, 'inputs': inputs}}

    if view.use_curve_mapping:
        if on_substitution is None:
            raise ValueError('display curve mapping requires a reported change to linear outputs')
        if emit_compositor is None:
            raise ValueError('display curve mapping requires a compositor writer')
        try:
            source = compositor_source(view.curve_mapping)
        except ValueError as error:
            on_substitution('use_curve_mapping', str(error),
                            'omit curve mapping; retain authored display settings')
        else:
            result['compositor'] = [emit_compositor(source)]
            on_substitution('use_curve_mapping',
                            'existing compositor runs before exposure, white balance and OCIO but also affects linear outputs',
                            'apply display curves to beauty including EXR/render images; alpha and controls unchanged')
    if view.use_white_balance:
        result.setdefault('compositor', []).append({
            'program': 'post/white_balance',
            'inputs': dict(temperature=view.white_balance_temperature, tint=view.white_balance_tint)})
    return result
