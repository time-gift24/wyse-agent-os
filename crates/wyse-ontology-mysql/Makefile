COMPOSE ?= podman compose
DATABASE_URL := mysql://ontology:ontology@127.0.0.1:33067/wyse_ontology

.PHONY: test-integration
test-integration:
	@set -e; \
	trap '$(COMPOSE) -f docker-compose.test.yml down -v' EXIT; \
	$(COMPOSE) -f docker-compose.test.yml up -d --wait; \
	DATABASE_URL=$(DATABASE_URL) sqlx migrate run --source migrations; \
	DATABASE_URL=$(DATABASE_URL) cargo test -p wyse-ontology-mysql --test mysql_repository -- --ignored
