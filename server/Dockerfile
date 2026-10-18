FROM python:3.12-slim
WORKDIR /app
ENV PYTHONUNBUFFERED=1 PYTHONDONTWRITEBYTECODE=1 HOME=/tmp
COPY --chmod=644 requirements.txt ./
RUN pip install --no-cache-dir -r requirements.txt
COPY --chmod=644 server.py ./
COPY repo_tweaks_server ./repo_tweaks_server
RUN chmod -R a=rX /app
USER 10001:10001
EXPOSE 8000
CMD ["python", "server.py"]
