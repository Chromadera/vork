# SPDX-License-Identifier: GPL-3.0-or-later
"""Rigid orientation of evaluated cameras and planar emitters."""


def rotation(matrix, *, area=False):
    # A planar emitter's local Z scale is irrelevant to its emitting surface.
    # Blender quaternion conversion needs three nonzero axes; restore the plane
    # normal from surviving X/Y axes before removing scale.
    if area and matrix.col[2].to_3d().length_squared == 0:
        normal = matrix.col[0].to_3d().cross(matrix.col[1].to_3d())
        if normal.length_squared > 0:
            matrix = matrix.copy()
            normal.normalize()
            matrix.col[2] = (*normal, 0.)
    return matrix.decompose()[1]
