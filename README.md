# symcfg

`symcfg`는 설정 원본 디렉터리와 애플리케이션이 읽는 위치의 심볼릭 링크를 `symbolic.json`으로 관리하는 CLI 도구입니다.

예를 들어 원본은 `~/config`에 두고, 실제 앱 위치에는 링크를 둘 수 있습니다.

```text
~/.config/nvim/init.lua -> ~/config/nvim/init.lua
~/.config/git/config   -> ~/config/git/config
```

## 설정 파일

기본 설정 파일은 현재 디렉터리의 `symbolic.json`입니다.

```json
{
  "version": 1,
  "links": [
    {
      "link": "~/.config/nvim/init.lua",
      "src": "nvim/init.lua"
    }
  ]
}
```

- `link`: 애플리케이션이 읽는 심볼릭 링크 위치입니다. 홈 아래 경로는 `~`로 저장됩니다.
- `src`: 실제 설정 원본 위치입니다. 명령을 실행한 디렉터리 아래 경로면 상대 경로로 저장됩니다.

보통 설정 저장소 루트에서 실행합니다.

```sh
cd ~/config
symcfg search ~/.config
symcfg apply --yes
```

명령 결과는 상태 라벨이 색으로 구분되며, 항목별 결과를 출력한 뒤 마지막에 요약을 출력합니다.

## 설치

Unix 계열 시스템에서 동작합니다.

```sh
cargo build --release
```

실행 파일은 `target/release/symcfg`에 생성됩니다.

## 명령

```sh
symcfg search <link-root>... [--source <source-root>] [-o symbolic.json]
```

`link-root` 아래에서 `source-root` 아래 원본을 가리키는 심볼릭 링크를 찾아 설정 파일에 추가합니다. `--source` 기본값은 `.`입니다.
각 항목은 `added`, `duplicate`, `conflict` 중 하나로 표시됩니다.

```sh
symcfg link <src> <link> [-c symbolic.json] [-y]
```

`src`를 가리키는 `link`를 만들고 설정 파일에 등록합니다. 부모 디렉터리가 없으면 확인하며, `--yes`를 주면 생성합니다.

```sh
symcfg apply [-c symbolic.json] [-y]
```

설정 파일에 기록된 링크를 생성합니다. 누락된 링크 부모 디렉터리는 만들지 않고 건너뜁니다.

```sh
symcfg list [-c symbolic.json]
```

설정 항목을 한 줄에 하나씩 출력합니다. 상태는 `linked`, `missing`, `conflict` 중 하나입니다.

```text
linked             ~/.config/nvim/init.lua -> nvim/init.lua
missing            ~/.config/git/config -> git/config
conflict           ~/.zshrc -> zsh/zshrc
```

```sh
symcfg sync [source-root] [-c symbolic.json] [-y] [--delete-links|--keep-links]
```

`source-root` 아래의 사라진 `src` 항목을 설정 파일에서 제거합니다. `source-root` 기본값은 `.`입니다. `--delete-links`는 기록된 `src`를 가리키는 심볼릭 링크만 삭제합니다.

```sh
symcfg validate [-c symbolic.json]
```

설정 파일 형식과 버전을 검증합니다.

## 안전 규칙

- 기존 일반 파일은 덮어쓰지 않습니다.
- 다른 원본을 가리키는 기존 링크는 충돌로 처리합니다.
- `apply`는 링크 부모 디렉터리를 자동 생성하지 않습니다.
- `sync --delete-links`는 설정과 일치하는 심볼릭 링크만 삭제합니다.

## 라이선스

MIT
