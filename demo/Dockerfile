FROM python:3.12-slim

WORKDIR /app

# Install dependencies (includes pytest for testing)
COPY requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt

# Copy application files
COPY app.py .
COPY test_app.py .
COPY .env .env

# Expose port: 8889 (App Server /events)
EXPOSE 8889

CMD ["uvicorn", "app:app", "--host", "0.0.0.0", "--port", "8889"]
