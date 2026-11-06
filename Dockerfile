# syntax=docker/dockerfile:1

# Reuse the upstream, published Tomato WebUI image for its compiled Rust server.
# Override at build time if you want to pin a tested upstream image tag/digest.
ARG TOMATO_IMAGE=zhongbai233/tomato-novel-downloader-webui:latest
FROM ${TOMATO_IMAGE} AS tomato-runtime

FROM python:3.11-slim-bookworm

ENV PYTHONUNBUFFERED=1 \
    PYTHONDONTWRITEBYTECODE=1 \
    HOME=/home/user \
    PATH=/home/user/.local/bin:$PATH \
    PYTHONPATH=/home/user/app \
    HF_SPACE_DEPLOYMENT=1 \
    NOVEL_DATA=/data/novelreader \
    NOVEL_MODELS=/data/models/translation \
    HACHIMIMT_MODELS_DIR=/data/models/translation \
    TTS_VOICE_DIR=/data/models/voices \
    NOVEL_OUTPUT_TEXT=/data/novelreader/output_text \
    NOVEL_OUTPUT_AUDIO=/data/novelreader/output_audio \
    HF_HOME=/data/cache/huggingface \
    HF_HUB_CACHE=/data/cache/huggingface/hub \
    TRANSFORMERS_CACHE=/data/cache/huggingface/transformers \
    XDG_CACHE_HOME=/data/cache \
    OMP_NUM_THREADS=2 \
    OPENBLAS_NUM_THREADS=2 \
    MKL_NUM_THREADS=2 \
    HACHIMIMT_BATCH_SIZE=4 \
    HACHIMIMT_WEB_BATCH_SIZE=4 \
    AUDIO_WORKERS=1 \
    TOKENIZERS_PARALLELISM=false \
    PIPER_DEVICE=cpu \
    PIPER_REQUIRE_CUDA=0 \
    ALLOWED_HOSTS=* \
    PORT=8000 \
    TOMATO_WEB_ADDR=127.0.0.1:18423 \
    TOMATO_WEB_URL=http://127.0.0.1:18423 \
    TOMATO_DATA_DIR=/data/tomato

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates ffmpeg libgomp1 espeak-ng \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --uid 1000 user \
    && mkdir -p /data \
    && chmod 0777 /data

WORKDIR /home/user/app

COPY requirements-hf-cpu.txt /tmp/requirements-hf-cpu.txt
RUN pip install --no-cache-dir --upgrade pip \
    && pip install --no-cache-dir -r /tmp/requirements-hf-cpu.txt

COPY --from=tomato-runtime /app/tomato-novel-downloader /usr/local/bin/tomato-novel-downloader
COPY --chown=user:user . /home/user/app

RUN chmod 0755 /home/user/app/hf-space/entrypoint.sh

USER user

EXPOSE 8000

CMD ["/home/user/app/hf-space/entrypoint.sh"]
