.PHONY: coverage-test coverage-docs coverage-all

coverage-test:
	./scripts/tests/check-test-coverage.sh

coverage-docs:
	./scripts/tests/check-rustdocs-coverage.sh

coverage-all: coverage-test coverage-docs
