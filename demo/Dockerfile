FROM python:3.12-slim

WORKDIR /app

# Install dependencies
COPY requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt

# Copy application files
COPY app.py .
COPY .env .env

# Expose ports: 8889 (App Server /events) and 8888 (OAuth Callback)
EXPOSE 8889 8888

CMD ["uvicorn", "app:app", "--host", "0.0.0.0", "--port", "8889"]
