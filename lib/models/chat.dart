class Chat {
  final String chatId;
  final String name;
  final String lastMessage;
  final DateTime lastMessageTime;
  final String? avatarPath; // 本地头像路径
  final int unreadCount;

  Chat({
    required this.chatId,
    required this.name,
    required this.lastMessage,
    required this.lastMessageTime,
    this.avatarPath,
    required this.unreadCount,
  });

  Chat copyWith({
    String? chatId,
    String? name,
    String? lastMessage,
    DateTime? lastMessageTime,
    String? avatarPath,
    int? unreadCount,
  }) {
    return Chat(
      chatId: chatId ?? this.chatId,
      name: name ?? this.name,
      lastMessage: lastMessage ?? this.lastMessage,
      lastMessageTime: lastMessageTime ?? this.lastMessageTime,
      avatarPath: avatarPath ?? this.avatarPath,
      unreadCount: unreadCount ?? this.unreadCount,
    );
  }

  Map<String, dynamic> toJson() => {
        'chatId': chatId,
        'name': name,
        'lastMessage': lastMessage,
        'lastMessageTime': lastMessageTime.toIso8601String(),
        'avatarPath': avatarPath,
        'unreadCount': unreadCount,
      };

  factory Chat.fromJson(Map<String, dynamic> json) => Chat(
        chatId: json['chatId'] as String,
        name: json['name'] as String,
        lastMessage: json['lastMessage'] as String,
        lastMessageTime: DateTime.parse(json['lastMessageTime'] as String),
        avatarPath: json['avatarPath'] as String?,
        unreadCount: json['unreadCount'] as int,
      );
}
