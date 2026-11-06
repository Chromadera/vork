# SPDX-License-Identifier: GPL-3.0-or-later
"""Evaluated Blender light controls in the renderer's normalized light units."""


def color(data):
    return [float(c) for c in data.color]


def temperature(data):
    """Kelvin when use_temperature is on: vork multiplies color by std.color blackbody."""
    return float(data.temperature) if data.use_temperature else None


def energy(data):
    return float(data.energy)*2.**float(data.exposure)
