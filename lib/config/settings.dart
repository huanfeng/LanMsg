import 'package:shared_preferences/shared_preferences.dart';
import 'package:flutter/material.dart';

class SettingsService {
  static const String _windowWidthKey = 'window_width';
  static const String _windowHeightKey = 'window_height';

  static late final SharedPreferences _prefs;

  static Future<void> init() async {
    _prefs = await SharedPreferences.getInstance();
  }

  static Future<void> saveWindowSize(Size size) async {
    await _prefs.setDouble(_windowWidthKey, size.width);
    await _prefs.setDouble(_windowHeightKey, size.height);
  }

  static Size? getWindowSize() {
    final width = _prefs.getDouble(_windowWidthKey);
    final height = _prefs.getDouble(_windowHeightKey);

    if (width != null && height != null) {
      return Size(width, height);
    }
    return null;
  }
}
