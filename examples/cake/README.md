# Cake

A seven-second camera arc around a carrot cake, with a wooden plate, kettle,
depth of field and indoor daylight. The README still comes from this scene and camera.
The textures and meshes are fetched into your output directory, not Git
(about 300 MB). Python's standard library is sufficient for preparation.

```sh
python3 examples/cake/prepare.py /tmp/vork-cake
vork render /tmp/vork-cake/scene.json --profile std/path --at 3 --set resolution=320x180 --out /tmp/cake.png
vork render /tmp/vork-cake/scene.json --profile std/path --out /tmp/cake.mp4
```

[Credits and asset licences](CREDITS.md). A low-resolution still checks the scene;
it does not reproduce the final film's sampling budget or establish its render time.
