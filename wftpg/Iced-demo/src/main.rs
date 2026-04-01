use iced::widget::{button, column, container, row, scrollable, text, text_input, Column, Space};
use iced::{Element, Length, Task};

pub fn main() -> iced::Result {
    iced::application(
        move || (IcedDemo::new(), Task::none()),
        IcedDemo::update,
        IcedDemo::view,
    )
    .run()
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
enum Message {
    TextInputChanged(String),
    ButtonClicked(String),
    ShowModal,
    HideModal,
    Increment,
    Decrement,
}

struct IcedDemo {
    input_value: String,
    message_log: Vec<String>,
    show_modal: bool,
    counter: i32,
}

impl IcedDemo {
    fn new() -> Self {
        Self {
            input_value: String::new(),
            message_log: Vec::new(),
            show_modal: false,
            counter: 0,
        }
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::TextInputChanged(value) => {
                self.input_value = value;
            }
            Message::ButtonClicked(btn_name) => {
                self.message_log
                    .push(format!("按钮被点击：{}", btn_name));
            }
            Message::ShowModal => {
                self.show_modal = true;
            }
            Message::HideModal => {
                self.show_modal = false;
            }
            Message::Increment => {
                self.counter += 1;
            }
            Message::Decrement => {
                self.counter -= 1;
            }
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        let main_content = column![
            text("Iced Demo - 基础组件示例").size(30),
            rule(),
            
            text("文本输入示例:").size(20),
            text_input("输入一些内容...", &self.input_value)
                .on_input(Message::TextInputChanged)
                .padding(10),
            text(format!("当前输入：{}", self.input_value)),
            
            rule(),
            
            text("按钮示例:").size(20),
            row![
                button(text("点击我"))
                    .on_press(Message::ButtonClicked(String::from("主按钮")))
                    .padding(10),
                button(text("增加"))
                    .on_press(Message::Increment)
                    .padding(10),
                button(text("减少"))
                    .on_press(Message::Decrement)
                    .padding(10),
                button(text("显示模态框"))
                    .on_press(Message::ShowModal)
                    .padding(10),
            ]
            .spacing(10),
            text(format!("计数器：{}", self.counter)).size(25),
            
            rule(),
            
            text("消息日志:").size(20),
            scrollable(
                Column::with_children(
                    self.message_log
                        .iter()
                        .map(|msg| -> Element<Message> { text(msg).into() })
                        .collect::<Vec<_>>(),
                )
                .spacing(5)
                .padding(5),
            )
            .height(150.0),
        ]
        .spacing(10)
        .padding(20);

        // 如果模态框应该显示，则叠加显示
        if self.show_modal {
            let modal = self.view_modal();
            // 创建半透明背景
            let overlay = container(modal)
                .width(Length::Fill)
                .height(Length::Fill)
                .center_x(Length::Fill)
                .center_y(Length::Fill);
            
            // 将主内容和遮罩层组合
            column![
                main_content,
                container(overlay)
                    .width(Length::Fill)
                    .height(Length::Fill)
            ]
            .into()
        } else {
            main_content.into()
        }
    }

    #[allow(dead_code)]
    fn view_modal(&self) -> Element<'_, Message> {
        let modal_content = column![
            text("这是一个模态框!").size(25),
            text("模态框可以用于显示重要信息或确认对话框。"),
            row![
                button(text("关闭"))
                    .on_press(Message::HideModal)
                    .padding(10),
                button(text("取消"))
                    .on_press(Message::HideModal)
                    .padding(10),
            ]
            .spacing(10),
        ]
        .spacing(15)
        .padding(20);

        // 创建模态框容器，添加背景和边框
        container(modal_content)
            .width(Length::Fixed(400.0))
            .height(Length::Fixed(300.0))
            .style(|_| container::Style {
                background: Some(iced::Background::Color(iced::Color::from_rgb(0.95, 0.95, 0.95))),
                border: iced::border::Border {
                    radius: 10.0.into(),
                    width: 2.0,
                    color: iced::Color::from_rgb(0.3, 0.3, 0.3),
                },
                ..container::Style::default()
            })
            .into()
    }
}

fn rule() -> Element<'static, Message> {
    Space::new().height(Length::Fixed(10.0)).into()
}
