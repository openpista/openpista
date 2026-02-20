.PHONY: coverage-test coverage-docs coverage-all

coverage-test:
	./scripts/check-test-coverage.sh

coverage-docs:
	./scripts/check-rustdocs-coverage.sh

coverage-all: coverage-test coverage-docs
