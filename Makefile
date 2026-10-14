.PHONY: install uninstall remove-fmgr

# Куда cargo ставит бинарники.
CARGO_HOME ?= $(HOME)/.cargo
BIN := $(CARGO_HOME)/bin
# Пометка строки, которую install добавляет в настройки оболочки.
MARK := \# janus
# Пометка той же строки, пока программа называлась fmgr.
OLD_MARK := \# fmgr

# Ставит janus в ~/.cargo/bin и удаляет target: при `--path` cargo собирает
# в target проекта, а после установки сборка больше не нужна.
#
# Затем подключает к оболочке функцию `f`: она запускает janus и после выхода
# переходит в директорию активной панели. Сменить директорию оболочки может
# только сама оболочка, поэтому это функция, а не ссылка на бинарник.
# Строка добавляется один раз; файл дописывается, а не заменяется, так что
# ~/.zshrc, лежащий символической ссылкой (dotfiles), не ломается.
install:
	cargo install --path . --locked
	cargo clean
	@$(MAKE) --no-print-directory remove-fmgr
	@case "$$(basename "$$SHELL")" in \
		zsh)  rc="$$HOME/.zshrc";  line='eval "$$(janus --shell-init zsh)"  $(MARK)' ;; \
		bash) rc="$$HOME/.bashrc"; line='eval "$$(janus --shell-init bash)"  $(MARK)' ;; \
		fish) rc="$$HOME/.config/fish/conf.d/janus.fish"; line='janus --shell-init fish | source  $(MARK)' ;; \
		*) echo "оболочка $$SHELL не поддерживается: функцию f подключите вручную (janus --shell-init)"; exit 0 ;; \
	esac; \
	mkdir -p "$$(dirname "$$rc")"; \
	if grep -qxF "$$line" "$$rc" 2>/dev/null; then \
		echo "функция f уже подключена в $$rc"; \
	else \
		printf '\n%s\n' "$$line" >> "$$rc"; \
		echo "функция f подключена в $$rc — откройте новый терминал или выполните: source $$rc"; \
	fi

# Удаляет бинарник и строку подключения функции `f`.
# Настройки в ~/.config/janus не трогаются.
uninstall:
	cargo uninstall janus
	@for rc in "$$HOME/.zshrc" "$$HOME/.bashrc"; do \
		if [ -f "$$rc" ] && grep -qF '$(MARK)' "$$rc"; then \
			tmp="$$(mktemp)"; grep -vF '  $(MARK)' "$$rc" > "$$tmp"; cat "$$tmp" > "$$rc"; rm -f "$$tmp"; \
			echo "функция f отключена в $$rc"; \
		fi; \
	done
	@rm -f "$$HOME/.config/fish/conf.d/janus.fish"

# Убирает установку под прежним именем fmgr: бинарник, ссылку `f` на него
# (так функцию подключали самые первые версии) и строку подключения
# функции `f`, иначе оболочка определит `f` дважды. Настройки из
# ~/.config/fmgr программа сама скопирует в ~/.config/janus при первом запуске.
remove-fmgr:
	@if cargo install --list | grep -q '^fmgr '; then cargo uninstall fmgr; fi
	@if [ "$$(readlink "$(BIN)/f")" = "fmgr" ]; then rm "$(BIN)/f"; fi
	@for rc in "$$HOME/.zshrc" "$$HOME/.bashrc"; do \
		if [ -f "$$rc" ] && grep -qF '$(OLD_MARK)' "$$rc"; then \
			tmp="$$(mktemp)"; grep -vF '  $(OLD_MARK)' "$$rc" > "$$tmp"; cat "$$tmp" > "$$rc"; rm -f "$$tmp"; \
			echo "старая функция f (fmgr) отключена в $$rc"; \
		fi; \
	done
	@rm -f "$$HOME/.config/fish/conf.d/fmgr.fish"
