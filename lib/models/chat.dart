import 'package:objectbox/objectbox.dart';

@Entity()
class Chat {
  @Id()
  int id = 0;
  
  @Unique()
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
}
