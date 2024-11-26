import 'package:flutter/material.dart';
import '../models/chat.dart';
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
    // 根据屏幕宽度调整尺寸
    final screenWidth = MediaQuery.of(context).size.width;
    final isSmallScreen = screenWidth < 600;

    final double avatarSize = isSmallScreen ? 40 : 44;
    final double horizontalPadding = isSmallScreen ? 8 : 16;
    final double verticalPadding = isSmallScreen ? 4 : 8;

    // 模拟聊天数据
    final chats = [
      Chat(
        id: '1',
        name: '张三',
        lastMessage: '你好，在吗？',
        lastMessageTime: DateTime.now().subtract(const Duration(minutes: 5)),
        avatar: 'https://via.placeholder.com/50',
        unreadCount: 2,
      ),
      Chat(
        id: '2',
        name: '李四',
        lastMessage: '下午开会记得准时',
        lastMessageTime: DateTime.now().subtract(const Duration(hours: 1)),
        avatar: 'https://via.placeholder.com/50',
        unreadCount: 0,
      ),
      // 添加更多模拟数据...
    ];

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
              final isSelected = chat.id == selectedChatId;

              return InkWell(
                onTap: () {
                  onChatSelected(chat.id);
                  if (MediaQuery.of(context).size.width <= 800) {
                    Navigator.push(
                      context,
                      MaterialPageRoute(
                        builder: (context) => ChatDetail(chatId: chat.id),
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
