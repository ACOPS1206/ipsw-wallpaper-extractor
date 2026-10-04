import copy
import pathlib
import sys
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
from plist_config import configure_ios, validate_ios


class SceneConfigurationTests(unittest.TestCase):
    def fixture(self):
        return {'CFBundleIdentifier': 'dev.acops.ipswWallpaperExtractor',
                'UIApplicationSceneManifest': {
                    'UIApplicationSupportsMultipleScenes': False,
                    'UISceneConfigurations': {
                        'UIWindowSceneSessionRoleApplication': [{
                            'UISceneConfigurationName': 'flutter',
                            'UISceneDelegateClassName': 'Runner.SceneDelegate',
                            'UISceneStoryboardFile': 'Main'}]}}}

    def test_sharing_flags_do_not_change_nested_scene_configuration(self):
        info = self.fixture()
        original = copy.deepcopy(info['UIApplicationSceneManifest'])
        configure_ios(info)
        self.assertEqual(info['UIApplicationSceneManifest'], original)
        self.assertIs(info['UIFileSharingEnabled'], True)
        self.assertIs(info['LSSupportsOpeningDocumentsInPlace'], True)
        self.assertEqual(configure_ios(copy.deepcopy(info)), info)

    def test_repair_previously_generated_boolean_scene_roles(self):
        info = self.fixture()
        scene = info['UIApplicationSceneManifest']
        for container in (info, scene, scene['UISceneConfigurations'],
                          scene['UISceneConfigurations']['UIWindowSceneSessionRoleApplication'][0]):
            container.update({'UIFileSharingEnabled': True, 'LSSupportsOpeningDocumentsInPlace': True})
        with self.assertRaises(ValueError):
            validate_ios(info)
        configure_ios(info)
        self.assertEqual(scene, self.fixture()['UIApplicationSceneManifest'])

    def test_validator_rejects_boolean_role_even_without_sharing_name(self):
        info = configure_ios(self.fixture())
        info['UIApplicationSceneManifest']['UISceneConfigurations']['unexpected'] = True
        with self.assertRaisesRegex(ValueError, 'must contain an array'):
            validate_ios(info)


if __name__ == '__main__':
    unittest.main()
