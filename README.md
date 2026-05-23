# symcfg

`symcfg`는 여러 애플리케이션 설정 파일을 한 곳에 모아 두고, 각 애플리케이션이 기대하는 위치에는 심볼릭 링크를 만들어 관리하는 CLI 도구입니다. 기존 링크를 찾아 `symbolic.json` 설정 파일로 기록하고, 새 컴퓨터에서는 그 설정 파일을 읽어 같은 링크 구조를 다시 만들 수 있습니다.

## 어떤 문제를 해결하나요?

개발 환경을 오래 쓰다 보면 실제 설정 파일은 `~/config` 같은 한 디렉터리에 모아 두고, `~/.config/nvim/init.lua`, `~/.config/git/config`, `~/.zshrc` 같은 앱별 위치에는 심볼릭 링크를 두는 방식이 편합니다.

하지만 이 구조를 다른 컴퓨터로 옮기는 일은 번거롭습니다.

- 심볼릭 링크 자체를 복사하면 대상 경로가 새 컴퓨터와 맞지 않을 수 있습니다.
- 어떤 링크가 어느 원본 설정 파일을 가리키는지 직접 기억해야 합니다.
- 앱별 설정 위치는 흩어져 있고, 원본 설정 저장소는 따로 있어 동기화 상태를 확인하기 어렵습니다.

`symcfg`는 링크 관계를 JSON으로 저장합니다. 설정 파일 저장소는 Git 등으로 동기화하고, 새 환경에서는 `symcfg apply`로 앱별 위치에 링크를 다시 만들 수 있습니다.

## 핵심 개념

### `src`

`src`는 실제 설정 파일 또는 디렉터리의 위치입니다. 예를 들어 `~/config/nvim/init.lua`처럼 사용자가 버전 관리하거나 백업하는 원본 경로입니다.

### `link`

`link`는 애플리케이션이 읽는 심볼릭 링크 위치입니다. 예를 들어 Neovim이 읽는 `~/.config/nvim/init.lua`가 `~/config/nvim/init.lua`를 가리키는 링크라면, `link`는 `~/.config/nvim/init.lua`입니다.

### 기본 설정 파일: `symbolic.json`

기본 설정 파일 이름은 `symbolic.json`입니다. 별도 옵션을 주지 않으면 `search`, `link`, `apply`, `sync`, `validate`는 현재 작업 디렉터리의 `symbolic.json`을 사용합니다.

설정 스키마는 다음과 같습니다.

```json
{
  "version": 1,
  "links": [
    {
      "link": "~/.config/nvim/init.lua",
      "src": "~/config/nvim/init.lua"
    },
    {
      "link": "~/.config/git/config",
      "src": "~/config/git/config"
    }
  ]
}
```

필드 이름은 `link`와 `src`입니다. `target` 필드는 사용하지 않습니다.

### `~` 홈 마커

현재 `HOME` 아래의 절대 경로는 설정 파일에 저장될 때 `~`로 축약됩니다. 예를 들어 현재 홈 아래의 `config/git/config` 경로는 `~/config/git/config`처럼 저장됩니다.

이 덕분에 사용자 이름이나 홈 디렉터리 절대 경로가 다른 컴퓨터에서도 같은 `symbolic.json`을 사용할 수 있습니다. 명령 실행 시 `~`는 현재 컴퓨터의 `HOME`으로 다시 확장됩니다.

## 설치 및 빌드

소스에서 빌드하려면 Rust toolchain과 Cargo가 필요합니다.
현재 구현은 Unix 심볼릭 링크 API를 사용하므로 macOS와 Linux 같은 Unix 계열 시스템을 대상으로 합니다. Windows는 현재 지원하지 않습니다.

```sh
git clone https://github.com/cLazyZombie/symcfg.git
cd symcfg
cargo build --release
```

빌드된 실행 파일은 다음 위치에 생성됩니다.

```sh
./target/release/symcfg
```

원하는 경우 PATH에 복사하거나 셸 설정에 별칭을 추가해 사용할 수 있습니다.

## 명령 개요

```sh
symcfg search --in <link-root>... [--source <source-root>] [-o|--output symbolic.json]
```

`link-root` 아래를 탐색해 `source-root` 아래의 원본을 가리키는 심볼릭 링크를 찾고 설정 파일에 등록합니다. `--source`의 기본값은 현재 디렉터리(`.`)입니다.

```sh
symcfg link <src> <link> [-c|--config symbolic.json] [-y|--yes]
```

`src`를 가리키는 새 심볼릭 링크 `link`를 만들고 설정 파일에 등록합니다.

```sh
symcfg apply [-c|--config symbolic.json] [-y|--yes]
```

설정 파일에 기록된 링크를 현재 컴퓨터에 생성합니다.

```sh
symcfg sync [--source <source-root>] [-c|--config symbolic.json] [-y|--yes] [--delete-links|--keep-links]
```

설정 파일에서 `source-root` 아래에 있지만 더 이상 존재하지 않는 `src` 항목을 제거합니다. `--source`의 기본값은 현재 디렉터리(`.`)입니다.

```sh
symcfg validate [-c|--config symbolic.json]
```

설정 파일을 읽어 버전, JSON 형식, 필수 경로 필드를 검증합니다.

## 빠른 사용 예시

아래 예시는 실제 설정 원본을 `~/config`에 두고, 앱이 읽는 위치인 `~/.config` 아래에는 심볼릭 링크를 두는 흐름입니다.

### 1. 설정 원본 디렉터리 준비

예를 들어 원본 설정을 다음처럼 관리한다고 가정합니다.

```text
~/config/
  nvim/init.lua
  git/config
```

애플리케이션은 다음 위치를 읽습니다.

```text
~/.config/nvim/init.lua -> ~/config/nvim/init.lua
~/.config/git/config   -> ~/config/git/config
```

홈 디렉터리 밖에 별도 디스크나 컨테이너 마운트를 사용한다면 `/config` 같은 원본 루트도 사용할 수 있습니다.

```text
/config/
  nvim/init.lua
  git/config
```

### 2. 기존 심볼릭 링크 찾기

이미 `~/.config` 아래에 링크가 만들어져 있다면, 설정 원본 루트에서 다음 명령을 실행합니다.

```sh
cd ~/config
symcfg search --in ~/.config --source ~/config
```

`--source` 아래의 파일 또는 디렉터리를 가리키는 심볼릭 링크만 `symbolic.json`에 추가됩니다.

출력 예시는 다음과 같습니다.

```text
Search complete: matched=2, added=2, duplicate=0, conflict=0
```

결과 설정 파일은 대략 다음 형태입니다.

```json
{
  "version": 1,
  "links": [
    {
      "link": "~/.config/git/config",
      "src": "~/config/git/config"
    },
    {
      "link": "~/.config/nvim/init.lua",
      "src": "~/config/nvim/init.lua"
    }
  ]
}
```

여러 링크 루트를 한 번에 검사할 수도 있습니다.

```sh
symcfg search --in ~/.config ~/.local/bin --source ~/config -o symbolic.json
```

### 3. 새 링크 하나 만들고 등록하기

새 설정 파일을 추가할 때는 `link` 명령을 사용할 수 있습니다.

```sh
symcfg link ~/config/alacritty/alacritty.toml ~/.config/alacritty/alacritty.toml
```

이 명령은 다음 두 가지를 함께 수행합니다.

1. `~/.config/alacritty/alacritty.toml` 심볼릭 링크를 생성합니다.
2. 같은 관계를 `symbolic.json`에 등록합니다.

링크의 부모 디렉터리(`~/.config/alacritty`)가 없으면 기본적으로 확인 프롬프트가 뜹니다. 자동으로 부모 디렉터리를 만들려면 `--yes`를 사용합니다.

```sh
symcfg link ~/config/alacritty/alacritty.toml ~/.config/alacritty/alacritty.toml --yes
```

이미 같은 링크가 같은 원본을 가리키고 있으면 파일시스템 작업은 건너뛰고 중복 등록으로 처리됩니다. 같은 위치에 일반 파일이 있거나 다른 원본을 가리키는 링크가 있으면 덮어쓰지 않고 오류 또는 충돌로 처리됩니다.

### 4. 새 컴퓨터에서 적용하기

새 컴퓨터에서 설정 저장소를 받은 뒤, 링크를 생성하기 전에 먼저 검증합니다.

```sh
cd ~/config
symcfg validate
```

문제가 없으면 다음처럼 적용합니다.

```sh
symcfg apply --yes
```

`--yes`는 생성 여부 확인 프롬프트 없이 누락된 링크를 만듭니다. 단, 링크의 부모 디렉터리가 없으면 `apply`는 부모 디렉터리를 새로 만들지 않고 해당 항목을 건너뜁니다. 필요한 앱 디렉터리(`~/.config/nvim` 등)는 미리 만들어 두거나 앱이 생성하게 한 뒤 다시 실행하세요.

출력 예시는 다음과 같습니다.

```text
Apply complete: created=2, skipped=0, conflict=0
```

### 5. 삭제된 원본과 설정 파일 동기화하기

원본 설정 파일을 삭제했거나 이름을 바꾼 뒤에는 `sync`로 설정 파일에서 오래된 항목을 제거할 수 있습니다.

```sh
symcfg sync --source ~/config
```

대화형 모드에서는 오래된 항목마다 링크를 삭제할지 보존할지 묻습니다.

자동 실행에서는 `--yes`와 함께 정책을 명시해야 합니다.

```sh
symcfg sync --source ~/config --yes --keep-links
```

`--keep-links`는 `src`가 사라진 항목을 `symbolic.json`에서는 제거하지만, 파일시스템의 링크는 남겨 둡니다. 링크를 수동으로 확인하거나 나중에 정리하고 싶을 때 사용합니다.

```sh
symcfg sync --source ~/config --yes --delete-links
```

`--delete-links`는 `src`가 사라진 항목을 `symbolic.json`에서 제거하고, 해당 `link`가 여전히 기록된 `src`를 가리키는 심볼릭 링크일 때만 링크 파일도 삭제합니다. 같은 경로가 일반 파일로 바뀌었거나 다른 대상을 가리키는 링크로 바뀐 경우에는 삭제하지 않습니다.

### 6. 적용 전 검증하기

`symbolic.json`을 직접 수정했거나 다른 컴퓨터에서 가져온 뒤에는 적용 전에 검증하는 습관을 권장합니다.

```sh
symcfg validate -c symbolic.json
```

성공하면 다음처럼 출력됩니다.

```text
Config is valid
```

## 안전 동작

`symcfg`는 설정 파일과 링크를 다룰 때 다음 안전 규칙을 따릅니다.

- 기존 파일을 덮어쓰지 않습니다.
  - `link`는 같은 경로에 일반 파일이 있거나 다른 원본을 가리키는 링크가 있으면 실패합니다.
  - `apply`는 링크 위치에 파일이 있거나 다른 원본을 가리키는 링크가 있으면 충돌로 보고 생성하지 않습니다.
- `apply`는 누락된 링크의 부모 디렉터리가 없으면 해당 항목을 건너뜁니다. 부모 디렉터리를 자동 생성하지 않습니다.
- `link --yes`는 누락된 링크 부모 디렉터리를 확인 없이 생성합니다.
- `sync --delete-links`는 설정에 기록된 `link`가 설정에 기록된 `src`를 가리키는 심볼릭 링크일 때만 삭제합니다. 일반 파일이나 다른 대상을 가리키는 링크는 삭제하지 않습니다.
- `search`는 링크 루트 탐색 중 심볼릭 링크된 디렉터리를 따라 들어가지 않습니다. 링크 루트 아래의 심볼릭 링크 자체만 검사합니다.
- `search`의 `source-root` 포함 여부는 원본 경로를 실제 경로로 해석한 뒤 판단합니다. 원본이 심볼릭 링크를 거쳐 `source-root` 밖을 가리키는 경우 잘못 포함된 것으로 처리되지 않도록 하기 위한 안전 장치입니다.
- 설정 저장 시 현재 `HOME` 아래 경로는 `~`로 저장되어 개인 홈 절대 경로가 설정 파일에 남지 않습니다.

## 생성되는 설정 파일과 Git 관리

`symcfg`가 생성하는 기본 설정 파일은 `symbolic.json`입니다. 이 저장소 자체의 `.gitignore`에는 `symbolic.json`이 포함되어 있습니다. 저장소 개발 중 로컬 사용자가 만든 개인 링크 설정이 실수로 공개 저장소에 커밋되는 것을 막기 위해서입니다.

사용자 자신의 설정 저장소에서는 선택할 수 있습니다.

- 여러 컴퓨터에서 같은 링크 구조를 재현하고 싶다면 `symbolic.json`을 버전 관리하세요.
- 컴퓨터마다 링크 구조가 다르거나 개인 경로를 저장하고 싶지 않다면 버전 관리에서 제외하세요.
- 다른 파일명을 쓰고 싶다면 `-c|--config` 또는 `-o|--output` 옵션으로 지정할 수 있습니다.

## 라이선스

MIT
