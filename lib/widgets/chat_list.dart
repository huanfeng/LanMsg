import 'package:flutter/material.dart';
import 'package:flutter_screenutil/flutter_screenutil.dart';
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

  @override
  Widget build(BuildContext context) {
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
                  color: isSelected ? Theme.of(context).colorScheme.primaryContainer : null,
                  child: ListTile(
                    leading: CircleAvatar(
                      backgroundImage: NetworkImage(chat.avatar),
                      radius: 20.r,
                    ),
                    title: Text(
                      chat.name,
                      style: TextStyle(
                        fontSize: 14.sp,
                        fontWeight: FontWeight.w500,
                      ),
                    ),
                    subtitle: Text(
                      chat.lastMessage,
                      style: TextStyle(
                        fontSize: 12.sp,
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
                          style: TextStyle(
                            fontSize: 10.sp,
                            color: Colors.grey,
                          ),
                        ),
                        if (chat.unreadCount > 0)
                          Container(
                            margin: EdgeInsets.only(top: 4.h),
                            padding: EdgeInsets.symmetric(
                              horizontal: 6.w,
                              vertical: 2.h,
                            ),
                            decoration: BoxDecoration(
                              color: Theme.of(context).colorScheme.error,
                              borderRadius: BorderRadius.circular(10.r),
                            ),
                            child: Text(
                              chat.unreadCount.toString(),
                              style: TextStyle(
                                color: Colors.white,
                                fontSize: 12.sp,
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
}
