COMPOSE ?= podman compose
TEST_COMPOSE_FILE := docker-compose.test.yml

.PHONY: test-integration test-up test-down

test-integration:
	@set -eu; \
	trap '$(COMPOSE) -f $(TEST_COMPOSE_FILE) down -v' EXIT; \
	$(COMPOSE) -f $(TEST_COMPOSE_FILE) up -d; \
	cargo test -p wyse-infra --test event_stream_bus_nats nats_rejects_incompatible_existing_stream -- --ignored --exact; \
	cargo test -p wyse-infra --test event_stream_bus_nats nats_agent_replay_modes_and_isolation -- --ignored --exact; \
	cargo test -p wyse-infra --test event_stream_bus_nats nats_reports_expired_cursor -- --ignored --exact; \
	cargo test -p wyse-infra --test event_stream_bus_nats nats_malformed_payload_terminates_subscription -- --ignored --exact; \
	cargo test -p wyse-infra --test event_stream_bus_nats seed_file_backed_event_for_restart -- --ignored --exact; \
	$(COMPOSE) -f $(TEST_COMPOSE_FILE) restart nats-stream; \
	cargo test -p wyse-infra --test event_stream_bus_nats replay_file_backed_event_after_restart -- --ignored --exact

test-up:
	$(COMPOSE) -f $(TEST_COMPOSE_FILE) up -d

test-down:
	$(COMPOSE) -f $(TEST_COMPOSE_FILE) down -v
