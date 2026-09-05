#!/bin/sh
set -eu
case "$1" in
    info) exit 0 ;;
    compose)
        shift
        if [ "$1" = version ]; then exit 0; fi
        [ "$1" = --env-file ] || exit 10
        [ -f "$2" ] || exit 11
        shift 2
        [ "$1" = -f ] || exit 12
        [ -f "$2" ] || exit 13
        shift 2
        case "$1" in
            config|up|ps|logs|stop) exit 0 ;;
            exec) exit "${CHICK_TEST_HEALTH_EXIT:-0}" ;;
            port) echo '0.0.0.0:18080'; exit 0 ;;
        esac
        ;;
esac
exit 14
