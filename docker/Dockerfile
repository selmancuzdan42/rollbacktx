FROM nginx:1.27-alpine

# Nginx config
COPY nginx.conf /etc/nginx/conf.d/default.conf

# Static site
COPY index.html /usr/share/nginx/html/index.html

# .deb dosyaları için dizin (volume mount edilecek)
RUN mkdir -p /usr/share/nginx/html/releases

EXPOSE 80
