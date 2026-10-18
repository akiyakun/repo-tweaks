FROM python:3.12-slim
WORKDIR /app
ENV PYTHONUNBUFFERED=1 PYTHONDONTWRITEBYTECODE=1 HOME=/tmp \
    REPO_TWEAKS_DATA_DIR=/data
COPY --chmod=644 requirements.txt ./
RUN pip install --no-cache-dir -r requirements.txt
COPY --chmod=644 server.py ./
COPY repo_tweaks_server ./repo_tweaks_server
# /data is owned by the app user so a new named volume starts out writable for it
RUN chmod -R a=rX /app && mkdir /data && chown 10001:10001 /data
USER 10001:10001
EXPOSE 8000
CMD ["python", "server.py"]
