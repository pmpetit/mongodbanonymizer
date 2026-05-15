# manon Documentation Makefile

# Documentation generation
.PHONY: install serve build clean

install:
	pip install -r requirements.txt

serve:
	mkdocs serve --config-file ../mkdocs.yml

build:
	mkdocs build --config-file ../mkdocs.yml

clean:
	rm -rf ../site/
