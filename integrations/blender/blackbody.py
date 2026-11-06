# SPDX-License-Identifier: GPL-3.0-or-later
"""Evaluate static Blackbody colour with Blender's native colour management.

Constant Blackbody nodes and Light.temperature_color share Blender's
IMB_colormanagement_blackbody_temperature_to_rgb evaluator. Query it through bpy
instead of shipping temperature coefficients or fitting a GPU ramp. Below 800K,
the light property's hard limit prevents querying the node's separate endpoint.
"""


def blackbody(temperature):
    import math
    if not math.isfinite(temperature):
        raise ValueError('Blackbody temperature must be finite')
    if temperature < 800:
        raise ValueError('Blackbody temperatures below 800K are outside the native light-evaluator subset; render this material in Blender')
    import bpy
    light = bpy.data.lights.new('vork temporary Blackbody evaluator', 'SUN')
    try:
        if not hasattr(light, 'temperature_color'):
            raise ValueError('Blackbody requires Blender exposing Light.temperature_color')
        light.use_temperature = True
        # The native Blackbody evaluator has a constant upper endpoint at 12000K.
        light.temperature = min(temperature, 12000)
        return list(light.temperature_color) + [1]
    finally:
        bpy.data.lights.remove(light)
