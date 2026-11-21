#!/usr/bin/env python3
"""Generate the demo fixture: a sphere PLY + 768-d CLIP-like features.

The query highlights the +x hemisphere, so the render should be yellow there.
"""
from pathlib import Path

import numpy as np

OUT = Path(__file__).resolve().parent

rng = np.random.default_rng(7)
n = 200
theta = np.pi * (np.arange(n) + 0.5) / n
phi = 2 * np.pi * (np.arange(n) + 0.5) / n
T, P = np.meshgrid(theta, phi, indexing="ij")
pts = np.stack(
    [np.sin(T) * np.cos(P), np.sin(T) * np.sin(P), np.cos(T)], 1
).astype(np.float32).reshape(-1, 3)

rgb = np.full((pts.shape[0], 3), 170, np.uint8)
rgb[np.abs(pts[:, 2]) < 0.03] = [210, 60, 60]

with open(OUT / "sphere.ply", "w") as f:
    f.write(
        "ply\nformat ascii 1.0\nelement vertex %d\n"
        "property float x\nproperty float y\nproperty float z\n"
        "property uchar red\nproperty uchar green\nproperty uchar blue\nend_header\n"
        % pts.shape[0]
    )
    for p, c in zip(pts, rgb):
        f.write(f"{p[0]:.6f} {p[1]:.6f} {p[2]:.6f} {c[0]} {c[1]} {c[2]}\n")

feat = 0.01 * rng.standard_normal((pts.shape[0], 768)).astype(np.float32)
feat[:, 0] = np.where(pts[:, 0] > 0.25, 1.0, 0.02)
np.save(OUT / "feats.npy", feat.astype(np.float16))

q = np.zeros((1, 768), np.float32)
q[0, 0] = 1.0
np.save(OUT / "query.npy", q)
print(f"{pts.shape[0]} points, features {feat.shape}")
