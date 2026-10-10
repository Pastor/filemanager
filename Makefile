.PHONY: install uninstall

# Куда cargo ставит бинарники.
CARGO_HOME ?= $(HOME)/.cargo
BIN := $(CARGO_HOME)/bin
# Короткое имя для запуска.
ALIAS := f

# Ставит fmgr в ~/.cargo/bin и удаляет target: при `--path` cargo собирает
# в target проекта, а после установки сборка больше не нужна.
# Алиас — символическая ссылка рядом с бинарником: работает в любой оболочке
# и не требует правки ~/.zshrc. Чужой файл с тем же именем не перезаписывается.
install:
	cargo install --path . --locked
	cargo clean
	@if [ -e "$(BIN)/$(ALIAS)" ] && [ "$$(readlink "$(BIN)/$(ALIAS)")" != "fmgr" ]; then \
		echo "$(BIN)/$(ALIAS) уже существует и это не ссылка на fmgr — алиас не создан"; \
		exit 1; \
	fi
	ln -sfn fmgr "$(BIN)/$(ALIAS)"

# Удаляет установленный бинарник и алиас. Настройки в ~/.config/fmgr не трогаются.
uninstall:
	cargo uninstall fmgr
	@if [ "$$(readlink "$(BIN)/$(ALIAS)")" = "fmgr" ]; then rm "$(BIN)/$(ALIAS)"; fi
