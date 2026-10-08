FROM nginx:alpine
RUN apk add --no-cache iproute2-tc
COPY --chmod=755 shape.sh /docker-entrypoint.d/05-shape.sh
COPY --chmod=755 reload.sh /docker-entrypoint.d/90-reload.sh
