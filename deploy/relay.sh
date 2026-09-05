#!/bin/sh
set -eu

DEPLOY_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
cd "$DEPLOY_DIR"
ACTION=${1:-help}
case "$ACTION" in
    init|up|start|status|logs|stop|check) ;;
    help|--help|-h)
        echo "Usage: sh deploy/relay.sh {init|up|start|status|logs|stop|check}"
        echo "First run: sh deploy/relay.sh up (default public port: 18080)"
        exit 0 ;;
    *) echo "Unknown command: $ACTION" >&2; exit 2 ;;
esac

initialize() {
    command -v openssl >/dev/null 2>&1 || {
        echo "OpenSSL is required to generate relay keys." >&2; exit 1;
    }
    if [ ! -f .env ]; then
        (umask 077; cp .env.example .env)
        echo "Created deploy/.env; edit RELAY_PUBLIC_PORT to change the public port."
    fi
    count=0
    for file in relay-private.pem relay-public.pem access-tokens.json client-tokens.env; do
        if [ -s "secrets/$file" ]; then count=$((count + 1)); fi
    done
    case "$count" in
        0) sh ./init-encrypted-direct.sh ;;
        4) echo "Keeping existing relay keys and owner tokens." ;;
        *) echo "Incomplete secrets directory. Restore missing files from backup; refusing to rotate identities." >&2; exit 1 ;;
    esac
}

if [ "$ACTION" = init ]; then initialize; exit 0; fi
command -v docker >/dev/null 2>&1 || {
    echo "Docker is required. Install Docker Engine and Compose, then rerun this command." >&2; exit 1;
}
if docker compose version >/dev/null 2>&1; then
    COMPOSE_MODE=plugin
elif command -v docker-compose >/dev/null 2>&1; then
    COMPOSE_MODE=standalone
else
    echo "Docker Compose is missing (docker compose or docker-compose)." >&2; exit 1
fi
compose() {
    if [ "$COMPOSE_MODE" = plugin ]; then
        docker compose --env-file "$DEPLOY_DIR/.env" \
            -f "$DEPLOY_DIR/docker-compose.encrypted-direct.yml" "$@"
    else
        docker-compose --env-file "$DEPLOY_DIR/.env" \
            -f "$DEPLOY_DIR/docker-compose.encrypted-direct.yml" "$@"
    fi
}
docker info >/dev/null 2>&1 || {
    echo "Docker daemon is unavailable or this account cannot access it." >&2; exit 1;
}
if [ "$ACTION" = up ] || [ "$ACTION" = start ]; then initialize; fi
[ -f .env ] || { echo "Run 'sh deploy/relay.sh init' first." >&2; exit 1; }
case "$ACTION" in
    up|start)
        compose config --quiet
        if [ "$ACTION" = start ]; then
            compose up -d --no-build
        else
            compose up -d --build
        fi
        attempts=0
        until compose exec -T relay curl --fail --silent --max-time 3 http://127.0.0.1:8080/healthz >/dev/null; do
            attempts=$((attempts + 1))
            if [ "$attempts" -ge 15 ]; then
                echo "Health check failed. Run: sh deploy/relay.sh logs" >&2; exit 1
            fi
            sleep 2
        done
        echo "Relay is healthy. Published port:"
        compose port relay 8080
        echo "Allow this TCP port in your cloud security group and host firewall."
        echo "Client setup and backup instructions: deploy/QUICKSTART.md"
        ;;
    status) compose ps ;;
    logs) compose logs --tail=100 --follow relay ;;
    stop) compose stop relay ;;
    check) compose exec -T relay curl --fail --silent --show-error --max-time 3 http://127.0.0.1:8080/healthz ;;
esac
