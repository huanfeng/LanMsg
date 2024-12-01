import 'package:flutter/material.dart';
import 'package:path/path.dart' as path;
import 'package:path_provider/path_provider.dart';
import 'dart:io';

class AvatarUtils {
  static const List<Color> _colors = [
    Color(0xFF1abc9c),
    Color(0xFF2ecc71),
    Color(0xFF3498db),
    Color(0xFF9b59b6),
    Color(0xFFe74c3c),
    Color(0xFFf1c40f),
    Color(0xFF34495e),
  ];

  // 缓存用户ID和颜色的映射
  static final Map<String, Color> _colorCache = {};

  /// 生成文字头像
  static Widget generateTextAvatar({
    required String text,
    required double size,
    String? userId,
  }) {
    final initials = text.isNotEmpty 
        ? text.split(' ').map((e) => e[0]).take(2).join().toUpperCase()
        : '?';
    
    // 使用用户ID来确定颜色，如果没有提供用户ID则使用文本
    final key = userId ?? text;
    final color = _colorCache[key] ?? _generateDeterministicColor(key);
    
    return Container(
      width: size,
      height: size,
      decoration: BoxDecoration(
        color: color,
        shape: BoxShape.circle,
      ),
      child: Center(
        child: Text(
          initials,
          style: TextStyle(
            color: Colors.white,
            fontSize: size * 0.4,
            fontWeight: FontWeight.bold,
          ),
        ),
      ),
    );
  }

  /// 基于字符串生成确定性的颜色
  static Color _generateDeterministicColor(String key) {
    if (_colorCache.containsKey(key)) {
      return _colorCache[key]!;
    }

    // 使用字符串的哈希值来确定颜色索引
    final index = key.hashCode.abs() % _colors.length;
    final color = _colors[index];
    
    // 缓存结果
    _colorCache[key] = color;
    return color;
  }

  /// 获取本地头像路径
  static Future<String> getLocalAvatarPath(String userId) async {
    final dir = await getApplicationDocumentsDirectory();
    final avatarsDir = Directory(path.join(dir.path, 'avatars'));
    if (!await avatarsDir.exists()) {
      await avatarsDir.create(recursive: true);
    }
    return path.join(avatarsDir.path, '$userId.png');
  }

  /// 保存头像到本地
  static Future<String> saveAvatarToLocal(String userId, File imageFile) async {
    final avatarPath = await getLocalAvatarPath(userId);
    await imageFile.copy(avatarPath);
    return avatarPath;
  }

  /// 获取头像Widget
  static Widget getAvatar({
    required String userId,
    required String name,
    String? avatarPath,
    double size = 40,
  }) {
    if (avatarPath != null && File(avatarPath).existsSync()) {
      return CircleAvatar(
        backgroundImage: FileImage(File(avatarPath)),
        radius: size / 2,
      );
    }
    
    return generateTextAvatar(
      text: name,
      size: size,
      userId: userId, // 传入userId以确保颜色一致性
    );
  }
}
