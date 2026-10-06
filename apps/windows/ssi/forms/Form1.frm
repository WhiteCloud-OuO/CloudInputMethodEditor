#VisualFreeBasic_Form#  Version=5.8.4
Locked=0

[Form]
Name=Form1
ClassStyle=CS_VREDRAW,CS_HREDRAW,CS_DBLCLKS
ClassName=
WinStyle=WS_CAPTION,WS_CLIPCHILDREN,WS_CLIPSIBLINGS,WS_MINIMIZEBOX,WS_SYSMENU,WS_VISIBLE,WS_EX_CONTROLPARENT,WS_EX_LEFT,WS_EX_LTRREADING,WS_EX_RIGHTSCROLLBAR,WS_POPUP
Style=3 - 常规窗口
Icon=icon.ico
Caption=特殊字符输入器
StartPosition=1 - 屏幕中心
WindowState=0 - 正常
Enabled=True
Repeat=False
Left=0
Top=0
Width=481
Height=610
TopMost=True
Child=False
MdiChild=False
TitleBar=True
SizeBox=False
SysMenu=True
MaximizeBox=False
MinimizeBox=True
Help=False
Hscroll=False
Vscroll=False
MinWidth=0
MinHeight=0
MaxWidth=0
MaxHeight=0
NoActivate=False
MousePass=False
TransPer=0
TransColor=SYS,25
Shadow=0 - 无阴影
BackColor=SYS,15
MousePointer=0 - 默认
Tag=
Tab=True
ToolTip=
ToolTipBalloon=False
AcceptFiles=False

[TabControl]
Name=TabControl1
Index=-1
Style=0 - 标签在顶部
Custom=6|0|常用符号||Form2|0|序号和角标||Form3|0|希腊字母||Form4|0|西里尔字母||Form5|0|日文假名||Form6|0|制表符||Form7|0|
FixedWidth=False
Multiline=False
Buttons=False
NoScroll=False
MultiSelect=False
FlatButton=False
IconLeft=False
LabelLeft=False
HotTrack=False
FocusButton=False
FocusNever=False
ToolTips=False
OwnDraw=False
HeaderTopPadding=4
HeaderSidePadding=4
TabHeight=22
TabWidth=40
Enabled=True
Visible=True
Left=4
Top=5
Width=454
Height=570
Layout=0 - 不锚定
MousePointer=0 - 默认
Tag=
Tab=True
ToolTip=
ToolTipBalloon=False
AcceptFiles=False


[AllCode]
'==================== 主窗口事件 ====================

'窗口与所有控件都已创建、但窗口尚未显示时调用（在这里把界面摆好，可避免显示后再调整造成闪烁）
Sub Form1_WM_Create(hWndForm As hWnd, UserData As Integer)
   TabControl1.Move 0,0,form1.ScaleWidth,form1.ScaleHeight
   TabControl1.SetSize
   '确保窗口置顶显示
   SetWindowPos hWndForm, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE Or SWP_NOSIZE
   '让本窗口“点击不激活”（不抢焦点），这样 SendInput 才能发到你原本打字的程序
   SetWindowLongPtr hWndForm, GWL_EXSTYLE, GetWindowLongPtr(hWndForm, GWL_EXSTYLE) Or WS_EX_NOACTIVATE
   '按当前选中的选项卡设置各子窗口的显示/隐藏（窗口显示前完成，避免闪一下）
   Dim sel As Long = TabControl1.Selected
   If sel < 0 Then sel = 0
   Form2.Visible = (sel = 0)
   Form3.Visible = (sel = 1)
   Form4.Visible = (sel = 2)
   Form5.Visible = (sel = 3)
   Form6.Visible = (sel = 4)
   Form7.Visible = (sel = 5)
   TabControl1.SetSize
End Sub

Sub Form1_Shown(hWndForm As hWnd,UserData As Integer)  '窗口完全显示后
   '界面已在 Form1_WM_Create 中摆好，这里不再做调整，避免闪烁
End Sub

'自定义消息：记录上一次的目标窗口，并让点击客户区不激活本窗口
Function Form1_Custom(hWndForm As hWnd, wMsg As UInteger, wParam As wParam, lParam As lParam) As LResult
   '记录“上一次的目标窗口”：激活状态变化时，lParam 就是另一个窗口（用户原本在用的程序）
   If wMsg = WM_ACTIVATE Then
      SetInputTarget Cast(HWND, lParam)
   End If
   '点击客户区时不激活本窗口，尽量不抢目标程序的键盘焦点
   If wMsg = WM_MOUSEACTIVATE Then
      If HiWord(lParam) = HTCLIENT Then Return MA_NOACTIVATE
   End If
   Function = False
End Function
