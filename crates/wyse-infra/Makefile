COMPOSE ?= podman compose
TEST_COMPOSE_FILE := docker-compose.test.yml

.PHONY: test-integration test-up test-down

test-integration:
	$(COMPOSE) -f $(TEST_COMPOSE_FILE) up -d
	cargo test -p wyse-infra --test event_stream_bus_nats -- --ignored --nocapture; \
	code=$$?; \
	$(COMPOSE) -f $(TEST_COMPOSE_FILE) down -v; \
	exit $$code

test-up:
	$(COMPOSE) -f $(TEST_COMPOSE_FILE) up -d

test-down:
	$(COMPOSE) -f $(TEST_COMPOSE_FILE) down -v
