class Chat {
  final String chatId;
  final String name;
  final String lastMessage;
  final DateTime lastMessageTime;
  final String avatar;
  final int unreadCount;

  Chat({
    required this.chatId,
    required this.name,
    required this.lastMessage,
    required this.lastMessageTime,
    required this.avatar,
    required this.unreadCount,
  });

  Chat copyWith({
    String? chatId,
    String? name,
    String? lastMessage,
    DateTime? lastMessageTime,
    String? avatar,
    int? unreadCount,
  }) {
    return Chat(
      chatId: chatId ?? this.chatId,
      name: name ?? this.name,
      lastMessage: lastMessage ?? this.lastMessage,
      lastMessageTime: lastMessageTime ?? this.lastMessageTime,
      avatar: avatar ?? this.avatar,
      unreadCount: unreadCount ?? this.unreadCount,
    );
  }

  Map<String, dynamic> toJson() => {
        'chatId': chatId,
        'name': name,
        'lastMessage': lastMessage,
        'lastMessageTime': lastMessageTime.toIso8601String(),
        'avatar': avatar,
        'unreadCount': unreadCount,
      };

  factory Chat.fromJson(Map<String, dynamic> json) => Chat(
        chatId: json['chatId'] as String,
        name: json['name'] as String,
        lastMessage: json['lastMessage'] as String,
        lastMessageTime: DateTime.parse(json['lastMessageTime'] as String),
        avatar: json['avatar'] as String,
        unreadCount: json['unreadCount'] as int,
      );
}
