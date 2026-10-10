.PHONY: install uninstall

# Куда cargo ставит бинарники.
CARGO_HOME ?= $(HOME)/.cargo
BIN := $(CARGO_HOME)/bin
# Пометка строки, которую install добавляет в настройки оболочки.
MARK := \# fmgr

# Ставит fmgr в ~/.cargo/bin и удаляет target: при `--path` cargo собирает
# в target проекта, а после установки сборка больше не нужна.
#
# Затем подключает к оболочке функцию `f`: она запускает fmgr и после выхода
# переходит в директорию активной панели. Сменить директорию оболочки может
# только сама оболочка, поэтому это функция, а не ссылка на бинарник.
# Строка добавляется один раз; файл дописывается, а не заменяется, так что
# ~/.zshrc, лежащий символической ссылкой (dotfiles), не ломается.
install:
	cargo install --path . --locked
	cargo clean
	@if [ "$$(readlink "$(BIN)/f")" = "fmgr" ]; then rm "$(BIN)/f"; fi
	@case "$$(basename "$$SHELL")" in \
		zsh)  rc="$$HOME/.zshrc";  line='eval "$$(fmgr --shell-init zsh)"  $(MARK)' ;; \
		bash) rc="$$HOME/.bashrc"; line='eval "$$(fmgr --shell-init bash)"  $(MARK)' ;; \
		fish) rc="$$HOME/.config/fish/conf.d/fmgr.fish"; line='fmgr --shell-init fish | source  $(MARK)' ;; \
		*) echo "оболочка $$SHELL не поддерживается: функцию f подключите вручную (fmgr --shell-init)"; exit 0 ;; \
	esac; \
	mkdir -p "$$(dirname "$$rc")"; \
	if grep -qxF "$$line" "$$rc" 2>/dev/null; then \
		echo "функция f уже подключена в $$rc"; \
	else \
		printf '\n%s\n' "$$line" >> "$$rc"; \
		echo "функция f подключена в $$rc — откройте новый терминал или выполните: source $$rc"; \
	fi

# Удаляет бинарник и строку подключения функции `f`.
# Настройки в ~/.config/fmgr не трогаются.
uninstall:
	cargo uninstall fmgr
	@if [ "$$(readlink "$(BIN)/f")" = "fmgr" ]; then rm "$(BIN)/f"; fi
	@for rc in "$$HOME/.zshrc" "$$HOME/.bashrc"; do \
		if [ -f "$$rc" ] && grep -qF '$(MARK)' "$$rc"; then \
			tmp="$$(mktemp)"; grep -vF '  $(MARK)' "$$rc" > "$$tmp"; cat "$$tmp" > "$$rc"; rm -f "$$tmp"; \
			echo "функция f отключена в $$rc"; \
		fi; \
	done
	@rm -f "$$HOME/.config/fish/conf.d/fmgr.fish"
