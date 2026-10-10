.PHONY: install uninstall

# Ставит fmgr в ~/.cargo/bin и удаляет target: при `--path` cargo собирает
# в target проекта, а после установки сборка больше не нужна.
install:
	cargo install --path . --locked
	cargo clean

# Удаляет установленный бинарник. Настройки в ~/.config/fmgr не трогаются.
uninstall:
	cargo uninstall fmgr
