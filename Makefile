# Oculus Makefile

.DEFAULT_GOAL := all

# =============================================================================
# Configuration
# =============================================================================
CARGO            := cargo
CARGO_FLAGS      := --locked
TAILWIND_BIN     := tailwindcss
TAILWIND_INPUT   := templates/static/tailwind.input.css
TAILWIND_OUTPUT  := templates/static/tailwind.css
TAILWIND_CONTENT := "templates/**/*.html src/**/*.rs"

# =============================================================================
# PHONY Targets
# =============================================================================
.PHONY: all fmt lint lint-rust lint-md check test doc doc-open build release install run tailwind tailwind-watch clean help

# =============================================================================
# Development Workflow
# =============================================================================

## Primary Targets
all: fmt lint check test build        ## Run full CI pipeline

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

build:                                ## Build debug binary
	@echo "Building debug binary..."
	@$(CARGO) build $(CARGO_FLAGS)

release:                              ## Build optimized release binary
	@echo "Building release binary..."
	@$(CARGO) build $(CARGO_FLAGS) --release

install:                              ## Install binary to ~/.cargo/bin
	@echo "Installing oculus..."
	@$(CARGO) install $(CARGO_FLAGS) --path .

run:                                  ## Run (use ARGS="..." for arguments)
	@$(CARGO) run $(CARGO_FLAGS) -- $(ARGS)

# =============================================================================
# Assets
# =============================================================================

tailwind:                             ## Build bundled Tailwind CSS (offline)
	@echo "Building Tailwind CSS..."
	@mkdir -p $(dir $(TAILWIND_OUTPUT))
	@test -f $(TAILWIND_INPUT) || { echo "Missing $(TAILWIND_INPUT); add @tailwind base/components/utilities"; exit 1; }
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
