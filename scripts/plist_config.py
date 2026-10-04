"""Configure Apple property lists structurally and validate scene role arrays."""
import pathlib
import plistlib
import sys

SHARING_KEYS = ('UIFileSharingEnabled', 'LSSupportsOpeningDocumentsInPlace')


def configure_ios(info):
    # Repair files produced by the old global </dict> replacement as well.
    def clean(value):
        if isinstance(value, dict):
            for key in SHARING_KEYS:
                value.pop(key, None)
            for child in value.values():
                clean(child)
        elif isinstance(value, list):
            for child in value:
                clean(child)
    clean(info)
    info.update({key: True for key in SHARING_KEYS})
    validate_ios(info)
    return info


def validate_ios(info):
    if not isinstance(info, dict):
        raise ValueError('Info.plist root must be a dictionary')
    for key in SHARING_KEYS:
        if info.get(key) is not True:
            raise ValueError(f'{key} must be true at the root')
    def check_nested(value):
        if isinstance(value, dict):
            if any(key in value for key in SHARING_KEYS):
                raise ValueError('File sharing flags must only appear at the root')
            for child in value.values():
                check_nested(child)
        elif isinstance(value, list):
            for child in value:
                check_nested(child)
    for value in info.values():
        check_nested(value)
    scene = info.get('UIApplicationSceneManifest')
    if scene is not None:
        if not isinstance(scene, dict):
            raise ValueError('UIApplicationSceneManifest must be a dictionary')
        roles = scene.get('UISceneConfigurations', {})
        if not isinstance(roles, dict):
            raise ValueError('UISceneConfigurations must be a dictionary')
        for role, configurations in roles.items():
            if not isinstance(configurations, list):
                raise ValueError(f'Scene role {role} must contain an array, not {type(configurations).__name__}')
            if any(not isinstance(item, dict) for item in configurations):
                raise ValueError(f'Scene role {role} has a non-dictionary configuration')


def write_plist(path, configure):
    path = pathlib.Path(path)
    with path.open('rb') as file:
        info = plistlib.load(file)
    configure(info)
    with path.open('wb') as file:
        plistlib.dump(info, file, sort_keys=False)


if __name__ == '__main__':
    if len(sys.argv) != 2:
        raise SystemExit('Usage: python3 scripts/plist_config.py <built Info.plist>')
    with open(sys.argv[1], 'rb') as file:
        validate_ios(plistlib.load(file))
    print('iOS Info.plist scene configuration and root sharing flags verified.')
