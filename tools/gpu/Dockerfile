# Context: tools/gpu/prepare.py creates it from a verified Linux release archive.
# No source checkout, credentials, NVIDIA driver or GPL FFmpeg build is copied.
FROM ubuntu:24.04 AS ffmpeg
RUN apt-get update && apt-get install -y --no-install-recommends build-essential curl ca-certificates xz-utils && rm -rf /var/lib/apt/lists/*
WORKDIR /src
RUN curl -fL https://ffmpeg.org/releases/ffmpeg-7.1.3.tar.xz -o ffmpeg.tar.xz && echo 'f0bf043299db9e3caacb435a712fc541fbb07df613c4b893e8b77e67baf3adbe  ffmpeg.tar.xz' | sha256sum -c - && tar xf ffmpeg.tar.xz
WORKDIR /src/ffmpeg-7.1.3
# LGPL-only, shared libraries so users can replace/relink them. H.264 encoding
# uses vork's native NVENC; this FFmpeg muxes it and supports input decoding.
RUN ./configure --prefix=/opt/ffmpeg --disable-gpl --disable-nonfree --disable-doc --disable-debug --disable-x86asm --enable-shared --disable-static && make -j4 && make install
RUN mkdir -p /opt/ffmpeg/notices && cp COPYING.LGPLv2.1 COPYING.LGPLv3 LICENSE.md /opt/ffmpeg/notices/ && cp /src/ffmpeg.tar.xz /opt/ffmpeg/notices/ && cp ffbuild/config.log /opt/ffmpeg/notices/
FROM ubuntu:24.04
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates libvulkan1 libegl1 libx11-6 libxcb1 libgomp1 libstdc++6 python3 && rm -rf /var/lib/apt/lists/*
COPY --from=ffmpeg /opt/ffmpeg /opt/ffmpeg
COPY bundle/ /opt/vork/
COPY entrypoint.sh /usr/local/bin/vork-container
COPY runner.py /opt/vork/remote-runner.py
ENV PATH=/opt/vork:/opt/ffmpeg/bin:$PATH LD_LIBRARY_PATH=/opt/ffmpeg/lib \
    VORK_SLANGC=/opt/vork/slang/bin/slangc VORK_OIDN=/opt/vork/oidn/lib/libOpenImageDenoise.so.2 \
    NVIDIA_VISIBLE_DEVICES=all NVIDIA_DRIVER_CAPABILITIES=graphics,compute,utility,video
WORKDIR /job
ENTRYPOINT ["/usr/local/bin/vork-container"]
CMD ["vork", "--help"]
