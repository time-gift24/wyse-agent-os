COMPOSE ?= podman compose
TEST_COMPOSE_FILE := docker-compose.test.yml
TEST_SANDBOX := $(CURDIR)/target/docker-sandbox

.PHONY: test-integration test-up test-down

test-integration:
	mkdir -p $(TEST_SANDBOX)
	$(COMPOSE) -f $(TEST_COMPOSE_FILE) up -d
	WYSE_TOOLS_DOCKER_SANDBOX=$(TEST_SANDBOX) cargo test -p wyse-tools --test apply_patch_docker -- --ignored --nocapture; \
	code=$$?; \
	$(COMPOSE) -f $(TEST_COMPOSE_FILE) down -v; \
	exit $$code

test-up:
	mkdir -p $(TEST_SANDBOX)
	$(COMPOSE) -f $(TEST_COMPOSE_FILE) up -d

test-down:
	$(COMPOSE) -f $(TEST_COMPOSE_FILE) down -v
