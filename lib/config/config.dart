import 'dart:io';

import 'package:path_provider/path_provider.dart';
import 'package:path/path.dart' as path;

class Config {
  static Future<String> getStorageLocation({bool autoCreate = true}) async {
    final documentDir = await getApplicationDocumentsDirectory();
    final myDir = Directory(path.join(documentDir.path, 'LanMsg'));
    if (autoCreate && !myDir.existsSync()) {
      myDir.createSync(recursive: true);
    }
    return myDir.path;
  }
}
