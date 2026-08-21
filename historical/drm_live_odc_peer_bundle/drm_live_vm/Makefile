.PHONY: test build run export clean

test:
	cargo test

build:
	docker compose build --pull

run:
	./scripts/run_peer.sh

export:
	./scripts/export_image.sh

clean:
	rm -rf peer-results artifacts target
