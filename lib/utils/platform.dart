import 'dart:io';

import 'package:flutter/services.dart';

bool checkPlatform(List<TargetPlatform> list) {
  for (var platform in list) {
    final isPlatform = switch (platform) {
      TargetPlatform.android => Platform.isAndroid,
      TargetPlatform.fuchsia => Platform.isFuchsia,
      TargetPlatform.iOS => Platform.isIOS,
      TargetPlatform.linux => Platform.isLinux,
      TargetPlatform.macOS => Platform.isMacOS,
      TargetPlatform.windows => Platform.isWindows,
    };
    if (isPlatform) {
      return true;
    }
  }
  return false;
}
