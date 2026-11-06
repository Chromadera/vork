# SPDX-License-Identifier: GPL-3.0-or-later
"""Blender camera sensor fit as vork's horizontal|vertical `fit`."""

def camera_fit(data, width, height):
    """vork camera fields for Blender's sensor_fit at this image size (pixel aspect applied).

    AUTO spans the longer image axis with sensor_width: horizontal in landscape,
    vertical with sensor_height = sensor_width in portrait.
    """
    if data.sensor_fit != 'AUTO':
        return {'fit': data.sensor_fit.lower()}
    if width >= height:
        return {'fit': 'horizontal'}
    return {'fit': 'vertical', 'sensor_height': data.sensor_width}
