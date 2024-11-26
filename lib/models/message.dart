import 'package:objectbox/objectbox.dart';

@Entity()
class Message {
  @Id()
  int id = 0;

  @Property()
  final String messageId;

  @Property()
  final String chatId;

  @Property()
  final String senderId;

  @Property()
  final String content;

  @Property()
  final DateTime timestamp;

  @Property()
  final bool isRead;

  Message({
    required this.messageId,
    required this.chatId,
    required this.senderId,
    required this.content,
    required this.timestamp,
    this.isRead = false,
  });

  Message copyWith({
    String? messageId,
    String? chatId,
    String? senderId,
    String? content,
    DateTime? timestamp,
    bool? isRead,
  }) {
    return Message(
      messageId: messageId ?? this.messageId,
      chatId: chatId ?? this.chatId,
      senderId: senderId ?? this.senderId,
      content: content ?? this.content,
      timestamp: timestamp ?? this.timestamp,
      isRead: isRead ?? this.isRead,
    );
  }
}
