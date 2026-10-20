#VisualFreeBasic_Form#  Version=5.8.4
Locked=0

[Form]
Name=Form1
ClassStyle=CS_VREDRAW,CS_HREDRAW,CS_DBLCLKS
ClassName=
WinStyle=WS_CAPTION,WS_CLIPCHILDREN,WS_CLIPSIBLINGS,WS_SYSMENU,WS_VISIBLE,WS_EX_CONTROLPARENT,WS_EX_LEFT,WS_EX_LTRREADING,WS_EX_RIGHTSCROLLBAR,WS_POPUP
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
TopMost=False
Child=False
MdiChild=False
TitleBar=True
SizeBox=False
SysMenu=True
MaximizeBox=False
MinimizeBox=False
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
Custom=7|0|常用符号||Form2|0|序号\|角标||Form3|0|希腊字母||Form4|0|西里尔字母||Form5|0|日文假名||Form6|0|制表符\|箭头||Form7|0|英文花体||Form8|0|
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
   ' 记录“上一次的目标窗口”
   If wMsg = WM_ACTIVATE Then
      SetInputTarget Cast(HWND, lParam)
   End If
   
   ' ★ 仅在“非 Form8 页面”时拦截鼠标激活
   If wMsg = WM_MOUSEACTIVATE Then
      Dim sel As Long = TabControl1.Selected
      If sel <> 6 Then          ' ★ 不是 Form8 页
         If HiWord(lParam) = HTCLIENT Then Return MA_NOACTIVATE
      End If
      ' 是 Form8 页时，不拦截，让窗口正常激活
   End If
   
   Function = False
End Function

'============================================================
' TabControl 切换页签时调用
'============================================================
Sub Form1_TabControl1_NM_Click(hWndForm As hWnd, hWndControl As hWnd)
   Dim sel As Long = TabControl1.Selected
   If sel < 0 Then sel = 0
   
   ' ---- 根据选中的页签决定激活策略 ----
   If sel = 6 Then   ' ★ 假设 Form8 是第 7 页（索引 6）
      ' 切到 Form8：允许抢焦点，并让 Form1 和 Text1 真正拿到焦点
      SetForm1ActivateMode True
      
      ' 如果窗口当前不是激活状态，把它带到前台并激活
      If GetForegroundWindow() <> hWndForm Then
         SetForegroundWindow hWndForm
      End If
      
      ' 把键盘焦点交给 Text1，这样才能输入
      SetFocus Form8.Text1.hWnd
   Else
      ' 切到其它页面：恢复“不抢焦点”模式
      SetForm1ActivateMode False
   End If
   
   ' ---- 原有的显示 / 隐藏子窗口逻辑（保持不变） ----
   Form2.Visible = (sel = 0)
   Form3.Visible = (sel = 1)
   Form4.Visible = (sel = 2)
   Form5.Visible = (sel = 3)
   Form6.Visible = (sel = 4)
   Form7.Visible = (sel = 5)
   Form8.Visible = (sel = 6)  ' 如果你的 Form8 是第 7 页，对应调整
   TabControl1.SetSize
End Sub

