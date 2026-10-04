# IPSW Wallpaper Extractor

IPSW 파일에서 Apple 배경 **원본 리소스**를 추출하는 Flutter + Rust 앱입니다. macOS, Windows, Linux, Android, iOS가 같은 엔진을 사용하며 휴대폰에서도 기기 내부에서 처리합니다. 서버 업로드, 탈옥, 루트 권한은 사용하지 않습니다.

## 사용

1. IPSW 파일을 선택하거나 내장 다운로더에서 기기·버전을 고릅니다.
2. 파일 시스템 이미지와 버전을 확인하고 원본 배경 추출을 누릅니다.
3. 추출한 리소스와 경로·크기·SHA-256이 기록된 `report.json`을 ZIP으로 저장하거나 공유합니다.

다운로더는 IPSW.me의 모델·버전 메타데이터를 이용하며 파일은 HTTPS Apple CDN에서만 받습니다. 직접 Apple CDN URL을 입력할 수도 있습니다. 취소·네트워크 중단 시 부분 파일을 유지하고 같은 항목을 다시 선택하면 이어받습니다. 카탈로그에 SHA-1이 제공되면 다운로드 후 검증합니다. 직접 URL에는 카탈로그 체크섬 검증이 없습니다. 추출을 위해 펌웨어가 서명 중일 필요는 없습니다.

## 지원 범위와 현재 한계

- BuildManifest의 OS/SystemOS/AppOS 이미지를 찾아 AEA → DMG → APFS/HFS+ 순서로 해석합니다. 보드를 선택하면 해당 BuildIdentity의 이미지만 처리합니다.
- Apple의 공개 FCS 서버에서 AEA profile 1 키를 가져옵니다. 오프라인 사용은 사용자가 제공한 Base64 대칭키가 필요합니다. 기타 AEA 프로필과 구형 암호화 DMG는 지원하지 않습니다.
- `/Library/Wallpaper`, `/System/Library/Wallpaper`, ProceduralWallpaper, WallpaperKit, Poster 확장 묶음의 파일과 디렉터리 구조를 보존합니다. 심볼릭 링크는 따라가지 않습니다. 새 출력 폴더만 사용하고 실패 시 임시 추출 결과를 정리합니다.
- **원본 PNG/JPEG/HEIF 추출과 셰이더 배경의 완성된 PNG 생성은 다른 기능입니다.** EXR·USDZ·Metal·CAML·Assets.car는 보존하지만 범용 렌더링, Assets.car 디코딩, `.tendies` 변환과 배경 설치는 아직 구현되지 않았습니다. 반사광·자이로 효과를 원본 PNG 하나로 추출했다고 표시하지 않습니다.
- 현재 구현은 AEA와 DMG를 임시 파일로 풀기 때문에 여유 저장공간이 이미지 크기의 수 배 필요할 수 있습니다. iOS 파일 선택기가 IPSW 사본을 만들 수도 있습니다. 모바일에서는 앱을 전면에 유지해야 하며 OS의 백그라운드 종료 후 자동 재개는 다운로드 부분 파일에만 적용됩니다.
- 대용량 파일의 I/O는 1 MiB 단위로 처리합니다. APFS/UDIF와 AEA 파서가 내부적으로 사용하는 메모리·지원 포맷은 의존 라이브러리의 제약을 따릅니다. 진행 상태는 현재 단계 기준이며 취소가 긴 파서 호출 중에는 지연될 수 있습니다.

## 빌드 및 Actions

`Build native apps` 워크플로는 Rust 테스트, Flutter 분석·위젯 테스트, Linux x64 / Windows x64 / macOS arm64·x64 / Android arm64·x64 APK / iOS arm64 IPA 빌드를 실행합니다. 아티팩트는 Actions 실행 화면에서 다운로드합니다.

- Android APK: 사이드로드용, Flutter 기본 디버그 서명입니다. Play Store 배포 서명은 별도입니다.
- iOS IPA: **미서명**이며 SideStore/AltStore 등으로 재서명이 필요합니다. Apple 개발자 계정 서명이나 App Store 배포 설정을 포함하지 않습니다.
- macOS 앱: ad-hoc 서명, 공증되지 않았습니다. 처음 실행할 때 시스템의 열기 승인이 필요할 수 있습니다.
- Linux: 아티팩트 ZIP을 푼 후 실행 파일에 실행 권한을 주고 실행합니다. GTK 3 런타임이 필요합니다.

플랫폼 러너는 설치된 Flutter stable 템플릿에서 생성됩니다. 앱 소스는 덮어쓰지 않으며 `scripts/scaffold.py`가 네트워크 권한, CMake 설치, iOS 정적 엔진 링크를 설정합니다.

```sh
cargo test --locked --manifest-path native/Cargo.toml
python3 scripts/scaffold.py
cd app
flutter analyze
flutter test
```

네이티브 엔진을 먼저 빌드·배치해야 실제 앱 실행이 가능합니다. 각 플랫폼의 정확한 빌드·배치 명령은 `.github/workflows/build.yml`을 따릅니다.

CLI:

```sh
cargo run --manifest-path native/Cargo.toml --bin ipsw-wallpaper -- \
  '{"op":"inspect","input":"firmware.ipsw"}'
cargo run --manifest-path native/Cargo.toml --bin ipsw-wallpaper -- \
  '{"op":"extract","input":"firmware.ipsw","output":"wallpapers"}'
```

## 의존성과 참고 자료

- [dpp](https://github.com/Dil4rd/dpp): DMG/APFS/HFS+ 읽기, MIT.
- [aea-tools](https://github.com/GavBog/aea-tools): AEA 읽기, MIT/Apache-2.0.
- [aeota](https://github.com/dhinakg/aeota): 공개 FCS 메타데이터·HPKE 방식 참고. 코드를 복사하지 않고 Rust로 구현.
- [IPSW.me API](https://api.ipsw.me/): 펌웨어 카탈로그.

Apple 배경이나 IPSW 파일은 저장소와 빌드 아티팩트에 포함하지 않습니다.
