import 'package:path_provider/path_provider.dart';
import 'package:path/path.dart' as path;

class Config {
  static Future<String> getHiveStorageLocation() async {
    final documentDir = await getApplicationDocumentsDirectory();
    return path.join(documentDir.path, 'LanMsg', 'hive');
  }
}
