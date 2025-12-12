# Oculus Makefile

.DEFAULT_GOAL := all

# =============================================================================
# Configuration
# =============================================================================
CARGO            := cargo
CARGO_FLAGS      := --locked
TAILWIND_BIN     := ./tools/tailwindcss
TAILWIND_INPUT   := templates/static/css/tailwind.input.css
TAILWIND_OUTPUT  := templates/static/css/tailwind.css
TAILWIND_CONTENT := "templates/**/*.html src/**/*.rs"

# =============================================================================
# PHONY Targets
# =============================================================================
.PHONY: all fmt lint lint-rust lint-md check test doc doc-open build release install run download-tailwind tailwind tailwind-watch clean help

# =============================================================================
# Development Workflow
# =============================================================================

## Primary Targets
all: fmt lint check test doc build    ## Run full CI pipeline

## Code Quality
fmt:                                  ## Format code with rustfmt
	@echo "Formatting code..."
	@$(CARGO) fmt

lint: lint-rust lint-md               ## Run all linters

lint-rust:                            ## Lint Rust code with clippy
	@echo "Linting Rust code..."
	@$(CARGO) clippy $(CARGO_FLAGS) --all-targets --all-features -- -D warnings

lint-md:                              ## Lint Markdown files
	@echo "Linting Markdown files..."
	@markdownlint .

check:                                ## Type-check without building
	@echo "Type-checking code..."
	@$(CARGO) check $(CARGO_FLAGS) --all-targets --all-features

# =============================================================================
# Testing
# =============================================================================

test:                                 ## Run all tests
	@echo "Running tests..."
	@$(CARGO) test $(CARGO_FLAGS) --all-targets --all-features

# =============================================================================
# Documentation
# =============================================================================

doc:                                  ## Build documentation
	@echo "Building documentation..."
	@$(CARGO) doc $(CARGO_FLAGS) --no-deps

doc-open:                             ## Build and open documentation
	@echo "Building and opening documentation..."
	@$(CARGO) doc $(CARGO_FLAGS) --no-deps --open

# =============================================================================
# Build & Run
# =============================================================================

build: tailwind                       ## Build debug binary
	@echo "Building debug binary..."
	@$(CARGO) build $(CARGO_FLAGS)

release: tailwind                     ## Build optimized release binary
	@echo "Building release binary..."
	@$(CARGO) build $(CARGO_FLAGS) --release

install: tailwind                      ## Install binary to ~/.cargo/bin
	@echo "Installing oculus..."
	@$(CARGO) install $(CARGO_FLAGS) --path .
run: tailwind                          ## Run (use ARGS="..." for arguments)
	@$(CARGO) run $(CARGO_FLAGS) -- $(ARGS)
# =============================================================================
# Assets
# =============================================================================

download-tailwind:                    ## Download Tailwind CSS CLI if not present
	@if [ ! -f $(TAILWIND_BIN) ]; then \
		echo "Downloading Tailwind CSS CLI..."; \
		mkdir -p tools; \
		curl -sL https://github.com/tailwindlabs/tailwindcss/releases/latest/download/tailwindcss-macos-arm64 -o $(TAILWIND_BIN); \
		chmod +x $(TAILWIND_BIN); \
		echo "✓ Tailwind CSS CLI installed to $(TAILWIND_BIN)"; \
	else \
		echo "✓ Tailwind CSS CLI already present"; \
	fi

tailwind: download-tailwind           ## Build bundled Tailwind CSS (offline)
	@echo "Building Tailwind CSS..."
	@mkdir -p $(dir $(TAILWIND_OUTPUT))
	@test -f $(TAILWIND_INPUT) || { echo "Missing $(TAILWIND_INPUT); add @import \"tailwindcss\""; exit 1; }
	@$(TAILWIND_BIN) -i $(TAILWIND_INPUT) -o $(TAILWIND_OUTPUT) --minify --content $(TAILWIND_CONTENT)

# =============================================================================
# Maintenance
# =============================================================================

clean:                                ## Remove build artifacts
	@echo "Cleaning build artifacts..."
	@$(CARGO) clean

update:                               ## Update dependencies
	@echo "Updating dependencies..."
	@$(CARGO) update

# =============================================================================
# Help
# =============================================================================

help:                                 ## Show available targets
	@echo "Oculus - Available targets:"
	@echo ""
	@awk 'BEGIN {FS = ":.*##"} /^[a-zA-Z_-]+:.*##/ { \
		printf "  \033[36m%-18s\033[0m %s\n", $$1, $$2 \
	}' $(MAKEFILE_LIST)
	@echo ""
	@echo "Examples:"
	@echo "  make                # Run full CI pipeline"
	@echo "  make run ARGS='--help'"
