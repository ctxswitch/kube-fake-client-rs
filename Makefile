.PHONY: help build test check clippy fmt clean doc pre-publish publish-dry publish install dev

.DEFAULT_GOAL := help

help: ## Show this help message
	@echo 'Usage: make [target]'
	@echo ''
	@echo 'Available targets:'
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-20s\033[0m %s\n", $$1, $$2}'

build: ## Build the project
	cargo build

build-release: ## Build the project in release mode
	cargo build --release

test: ## Run all tests
	cargo test

test-verbose: ## Run tests with verbose output
	cargo test -- --nocapture

check: ## Check the project for errors
	cargo check --all-targets

clippy: ## Run clippy linter
	cargo clippy --all-targets -- -D warnings

fmt: ## Format code with rustfmt
	cargo fmt

fmt-check: ## Check code formatting without modifying files
	cargo fmt -- --check

clean: ## Clean build artifacts
	cargo clean

doc: ## Generate documentation
	cargo doc --no-deps

doc-open: ## Generate and open documentation in browser
	cargo doc --no-deps --open

install: ## Install the package locally
	cargo install --path .

dev: fmt clippy test ## Run formatting, linting, and tests (development workflow)

watch-test: ## Watch for changes and run tests
	cargo watch -x test

watch-check: ## Watch for changes and run check
	cargo watch -x check

pre-publish: fmt-check clippy test ## Run all checks before publishing
	@echo "Running pre-publish checks..."
	@cargo package --list
	@echo ""
	@echo "Current version: $$(cargo pkgid | cut -d'#' -f2 | cut -d'@' -f2)"
	@echo ""
	@echo "✓ All pre-publish checks passed!"

publish-dry: pre-publish ## Dry-run publish to crates.io
	@echo "Running dry-run publish..."
	cargo publish --dry-run

publish: pre-publish ## Publish to crates.io (requires confirmation)
	@echo "This will publish version $$(cargo pkgid | cut -d'#' -f2 | cut -d'@' -f2) to crates.io"
	@echo "Have you:"
	@echo "  1. Updated CHANGELOG?"
	@echo "  2. Bumped version in Cargo.toml?"
	@echo "  3. Committed all changes?"
	@echo "  4. Created a git tag?"
	@echo ""
	@read -p "Continue? [y/N] " -n 1 -r; \
	echo; \
	if [[ $$REPLY =~ ^[Yy]$$ ]]; then \
		cargo publish; \
	else \
		echo "Publish cancelled."; \
		exit 1; \
	fi

tag: ## Create a git tag for the current version
	@VERSION=$$(cargo pkgid | cut -d'#' -f2 | cut -d'@' -f2); \
	echo "Creating tag v$$VERSION"; \
	git tag -a "v$$VERSION" -m "Release v$$VERSION"; \
	echo "Tag created. Push with: git push origin v$$VERSION"

changelog: ## Generate CHANGELOG.md from git commits
	@command -v git-cliff >/dev/null 2>&1 || { echo "Installing git-cliff..."; cargo install git-cliff; }
	git-cliff --output CHANGELOG.md
	@echo "✓ Generated CHANGELOG.md"

changelog-unreleased: ## Show unreleased changes
	@command -v git-cliff >/dev/null 2>&1 || { echo "Installing git-cliff..."; cargo install git-cliff; }
	git-cliff --unreleased

changelog-tag: ## Generate changelog for the current version tag
	@VERSION=$$(cargo pkgid | cut -d'#' -f2 | cut -d'@' -f2); \
	command -v git-cliff >/dev/null 2>&1 || { echo "Installing git-cliff..."; cargo install git-cliff; }; \
	git-cliff --latest --tag "v$$VERSION" --output RELEASE_NOTES.md; \
	echo "✓ Generated RELEASE_NOTES.md for v$$VERSION"

bench: ## Run benchmarks
	cargo bench

audit: ## Run security audit
	cargo audit

outdated: ## Check for outdated dependencies
	cargo outdated

update: ## Update dependencies
	cargo update

tree: ## Show dependency tree
	cargo tree

bloat: ## Analyze binary size
	cargo bloat --release

all: clean build test doc ## Clean, build, test, and generate docs
