import 'package:flutter/material.dart';
import 'package:provider/provider.dart';

import '../services/message_service.dart';
import 'chat_detail.dart';

class ChatList extends StatelessWidget {
  final Function(String) onChatSelected;
  final String? selectedChatId;

  const ChatList({
    super.key,
    required this.onChatSelected,
    this.selectedChatId,
  });

  String _formatTime(DateTime time) {
    final now = DateTime.now();
    final difference = now.difference(time);

    if (difference.inMinutes < 60) {
      return '${difference.inMinutes}分钟前';
    } else if (difference.inHours < 24) {
      return '${difference.inHours}小时前';
    } else if (difference.inDays < 7) {
      return '${difference.inDays}天前';
    } else {
      return '${time.month}-${time.day}';
    }
  }

  @override
  Widget build(BuildContext context) {
    final textTheme = Theme.of(context).textTheme;
    final screenWidth = MediaQuery.of(context).size.width;
    final isSmallScreen = screenWidth < 600;

    final double avatarSize = isSmallScreen ? 40 : 44;
    final double horizontalPadding = isSmallScreen ? 8 : 16;
    final double verticalPadding = isSmallScreen ? 4 : 8;

    final messageService = context.watch<MessageService>();
    final chats = messageService.getAllChats();

    return Column(
      children: [
        AppBar(
          title: const Text('消息'),
          centerTitle: false,
          actions: [
            IconButton(
              icon: const Icon(Icons.add_circle_outline),
              onPressed: () {
                // TODO: 实现添加新聊天功能
              },
            ),
          ],
        ),
        Expanded(
          child: ListView.builder(
            itemCount: chats.length,
            itemBuilder: (context, index) {
              final chat = chats[index];
              final isSelected = chat.chatId == selectedChatId;

              return InkWell(
                onTap: () {
                  onChatSelected(chat.chatId);
                  if (MediaQuery.of(context).size.width <= 800) {
                    Navigator.push(
                      context,
                      MaterialPageRoute(
                        builder: (context) => ChatDetail(chatId: chat.chatId),
                      ),
                    );
                  }
                },
                child: Container(
                  color: isSelected
                      ? Theme.of(context).colorScheme.primaryContainer
                      : null,
                  child: ListTile(
                    contentPadding: EdgeInsets.symmetric(
                      horizontal: horizontalPadding,
                      vertical: verticalPadding,
                    ),
                    leading: CircleAvatar(
                      backgroundImage: NetworkImage(chat.avatar),
                      radius: avatarSize / 2,
                    ),
                    title: Text(
                      chat.name,
                      style: textTheme.bodyLarge?.copyWith(
                        fontWeight: FontWeight.w500,
                      ),
                    ),
                    subtitle: Text(
                      chat.lastMessage,
                      style: textTheme.bodySmall?.copyWith(
                        color: Colors.grey[600],
                      ),
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                    ),
                    trailing: Column(
                      mainAxisAlignment: MainAxisAlignment.center,
                      crossAxisAlignment: CrossAxisAlignment.end,
                      children: [
                        Text(
                          _formatTime(chat.lastMessageTime),
                          style: textTheme.bodySmall?.copyWith(
                            color: Colors.grey,
                          ),
                        ),
                        if (chat.unreadCount > 0)
                          Container(
                            margin: const EdgeInsets.only(top: 4),
                            padding: const EdgeInsets.symmetric(
                              horizontal: 6,
                              vertical: 2,
                            ),
                            decoration: BoxDecoration(
                              color: Theme.of(context).colorScheme.error,
                              borderRadius: BorderRadius.circular(10),
                            ),
                            child: Text(
                              chat.unreadCount.toString(),
                              style: textTheme.labelSmall?.copyWith(
                                color: Colors.white,
                              ),
                            ),
                          ),
                      ],
                    ),
                  ),
                ),
              );
            },
          ),
        ),
      ],
    );
  }
}
