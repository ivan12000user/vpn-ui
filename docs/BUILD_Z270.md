# Сборка UI на Z270 WSL

Цель — собрать Linux amd64 UI из известного commit отдельно от работающей
лаборатории и VPS. Проверенная среда Z270: x86_64, glibc 2.35, около 8.8 GiB RAM.
Rust/Cargo пока не установлены. Исходники хранить в Linux filesystem под
`/home/user`, а не `/mnt/c`.

## Подготовка инструментов

```bash
sudo apt-get update
sudo apt-get install --yes build-essential pkg-config git curl ca-certificates python3
curl --proto '=https' --tlsv1.2 -fSs https://sh.rustup.rs -o /tmp/vpn-ui-rustup-init.sh
less /tmp/vpn-ui-rustup-init.sh
sh /tmp/vpn-ui-rustup-init.sh -y --profile minimal --default-toolchain 1.98.0
. "$HOME/.cargo/env"
rustup component add rustfmt --toolchain 1.98.0
```

Системный apt Rust 1.75 не подходит: проект требует 1.98.

## Одна рабочая копия

Первый clone (если каталог уже существует, сначала проверить его):

```bash
mkdir -p "$HOME/src"
git clone --branch integration/v080-awg31-tailscale-prep-20261011 \
  https://github.com/ivan12000user/vpn-ui.git "$HOME/src/vpn-ui"
cd "$HOME/src/vpn-ui"
git status --short --branch
git log -1 --format='%H %s'
```

Для воспроизведения аудированной исходной точки в чистой рабочей копии:

```bash
git switch --detach 87355316e618878516f8aec6a1c17cb37d3fce57
```

Для разработки использовать новую ветку от актуальной интеграции и записывать
фактический SHA. Не заменять бинарник лаборатории при clone или сборке.

## Проверки и сборка

```bash
cd "$HOME/src/vpn-ui"
. "$HOME/.cargo/env"
cargo +1.98.0 fmt --all -- --check
cargo +1.98.0 check --locked
cargo +1.98.0 test --locked
python3 -m unittest discover -s tests -p 'test_*.py' -v
cargo +1.98.0 build --release --locked
python3 tests/stage_deployment.py
bash tests/smoke_web_local.sh
```

Остановиться на первой ошибке и сохранить вывод. Smoke использует localhost
18790 с заглушками VPN-команд; действующая лаборатория работает на 18791.
Не запускать smoke, если 18790 занят. Не устанавливать deploy/ на Z270.
Offline staging не является проверкой настоящих VPN-туннелей.

## Сохранение идентичности сборки

После успешных проверок, в чистом checkout:

```bash
test -z "$(git status --porcelain)" || { echo 'Checkout contains changes'; exit 1; }
revision=$(git rev-parse HEAD)
artifact_dir="$HOME/vpn-ui-artifacts/$revision"
mkdir -p "$HOME/vpn-ui-artifacts"
mkdir "$artifact_dir"
install -m 0755 target/release/vpn-ui "$artifact_dir/vpn-ui"
printf '%s\n' "$revision" > "$artifact_dir/SOURCE_COMMIT"
{ uname -sm; getconf GNU_LIBC_VERSION; rustc +1.98.0 -Vv; } > "$artifact_dir/BUILD_PLATFORM"
(cd "$artifact_dir" && sha256sum vpn-ui > SHA256SUMS && sha256sum -c SHA256SUMS)
```

Каталог с тем же SHA не перезаписывается. Для повторной сборки выбрать отдельный
каталог и сравнить результаты. Эта процедура не устанавливает бинарник в
лабораторию или на VPS. Для VPS отдельно проверить GLIBC/архитектуру,
сохранить предыдущий бинарник и подготовить откат только UI.
