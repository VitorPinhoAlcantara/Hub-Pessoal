package main

import (
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"sort"
	"strings"
	"time"

	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/types"
	"go.mau.fi/whatsmeow/types/events"
	"google.golang.org/protobuf/proto"
)

const (
	CmdLogin             = "login"
	CmdListConversations = "listar_conversas"
	CmdListMessages      = "listar_mensagens"
	CmdSend              = "enviar"
	CmdChatInfo          = "info_conversas"
	CmdDownloadMedia     = "baixar_midia"
	CmdShutdown          = "encerrar"
)

const (
	EventQR              = "qr"
	EventConnected       = "conectado"
	EventMessageReceived = "mensagem_recebida"
	EventDisconnected    = "desconectado"
	EventArchived        = "arquivada"
	EventError           = "erro"
)

const (
	ReasonSessionEnded   = "sessao_encerrada"
	ReasonConnectionLost = "conexao_perdida"
)

const (
	KindText     = "texto"
	KindImage    = "imagem"
	KindAudio    = "audio"
	KindVideo    = "video"
	KindDocument = "documento"
	KindSticker  = "figurinha"
	KindOther    = "outro"
)

type Command struct {
	ID        int64    `json:"id"`
	Cmd       string   `json:"cmd"`
	JID       string   `json:"jid,omitempty"`
	Text      string   `json:"texto,omitempty"`
	Limit     int      `json:"limite,omitempty"`
	JIDs      []string `json:"jids,omitempty"`
	Media     string   `json:"midia,omitempty"`
	MessageID string   `json:"mensagem_id,omitempty"`
}

type rawCommand struct {
	ID        *int64   `json:"id"`
	Cmd       string   `json:"cmd"`
	JID       string   `json:"jid"`
	Text      string   `json:"texto"`
	Limit     int      `json:"limite"`
	JIDs      []string `json:"jids"`
	Media     string   `json:"midia"`
	MessageID string   `json:"mensagem_id"`
}

type OKResponse struct {
	ID   int64 `json:"id"`
	OK   bool  `json:"ok"`
	Data any   `json:"dados"`
}

type ErrorResponse struct {
	ID    int64  `json:"id"`
	OK    bool   `json:"ok"`
	Error string `json:"erro"`
}

type Event struct {
	Name     string `json:"evento"`
	Code     string `json:"codigo,omitempty"`
	JID      string `json:"jid,omitempty"`
	Message  any    `json:"mensagem,omitempty"`
	History  bool   `json:"historico,omitempty"`
	Reason   string `json:"motivo,omitempty"`
	Archived *bool  `json:"arquivada,omitempty"`
}

type Message struct {
	ID       string `json:"id"`
	Chat     string `json:"jid"`
	From     string `json:"de"`
	FromName string `json:"nome_de"`
	Text     string `json:"texto"`
	Mine     bool   `json:"minha"`
	At       int64  `json:"em"`
	Kind     string `json:"tipo"`
	Media    string `json:"midia,omitempty"`
}

type ChatInfo struct {
	JID      string `json:"jid"`
	Phone    string `json:"telefone,omitempty"`
	Name     string `json:"nome,omitempty"`
	Archived bool   `json:"arquivada"`
}

type DownloadResult struct {
	Path string `json:"caminho"`
}

type Conversation struct {
	JID   string `json:"jid"`
	Name  string `json:"nome"`
	Group bool   `json:"grupo"`
}

var errEmptyCommand = errors.New("comando vazio")

func decodeCommand(line []byte) (Command, error) {
	if len(strings.TrimSpace(string(line))) == 0 {
		return Command{}, errEmptyCommand
	}
	var raw rawCommand
	if err := json.Unmarshal(line, &raw); err != nil {
		return Command{}, fmt.Errorf("JSON inválido: %w", err)
	}
	if raw.ID == nil {
		return Command{}, errors.New("comando sem o campo id")
	}
	cmd := Command{
		ID:        *raw.ID,
		Cmd:       strings.TrimSpace(raw.Cmd),
		JID:       strings.TrimSpace(raw.JID),
		Text:      raw.Text,
		Limit:     raw.Limit,
		JIDs:      raw.JIDs,
		Media:     raw.Media,
		MessageID: strings.TrimSpace(raw.MessageID),
	}
	if cmd.Cmd == "" {
		return cmd, errors.New("comando sem o campo cmd")
	}
	return cmd, nil
}

func okResponse(id int64, data any) OKResponse {
	return OKResponse{ID: id, OK: true, Data: data}
}

func errorResponse(id int64, message string) ErrorResponse {
	return ErrorResponse{ID: id, OK: false, Error: message}
}

func qrEvent(code string) Event {
	return Event{Name: EventQR, Code: code}
}

func connectedEvent(jid string) Event {
	return Event{Name: EventConnected, JID: jid}
}

func messageEvent(msg Message, history bool) Event {
	return Event{Name: EventMessageReceived, Message: msg, History: history}
}

func disconnectedEvent(reason string) Event {
	return Event{Name: EventDisconnected, Reason: reason}
}

func errorEvent(message string) Event {
	return Event{Name: EventError, Message: message}
}

func archivedEvent(jid string, archived bool) Event {
	return Event{Name: EventArchived, JID: jid, Archived: &archived}
}

func encodeImage(img *waE2E.ImageMessage) string {
	if img == nil || img.GetDirectPath() == "" {
		return ""
	}
	clone := proto.Clone(img).(*waE2E.ImageMessage)
	clone.JPEGThumbnail = nil
	clone.ContextInfo = nil
	data, err := proto.Marshal(clone)
	if err != nil {
		return ""
	}
	return base64.StdEncoding.EncodeToString(data)
}

func decodeImage(media string) (*waE2E.ImageMessage, error) {
	data, err := base64.StdEncoding.DecodeString(media)
	if err != nil {
		return nil, fmt.Errorf("mídia inválida: %w", err)
	}
	var img waE2E.ImageMessage
	if err := proto.Unmarshal(data, &img); err != nil {
		return nil, fmt.Errorf("mídia inválida: %w", err)
	}
	if img.GetDirectPath() == "" {
		return nil, errors.New("mídia sem caminho de download")
	}
	return &img, nil
}

func mediaFileName(id, mimetype string) string {
	var name strings.Builder
	for _, r := range id {
		if (r >= 'a' && r <= 'z') || (r >= 'A' && r <= 'Z') || (r >= '0' && r <= '9') || r == '-' || r == '_' {
			name.WriteRune(r)
		}
	}
	if name.Len() == 0 {
		name.WriteString("imagem")
	}
	extension := ".jpg"
	switch strings.ToLower(strings.TrimSpace(strings.Split(mimetype, ";")[0])) {
	case "image/png":
		extension = ".png"
	case "image/webp":
		extension = ".webp"
	case "image/gif":
		extension = ".gif"
	}
	return name.String() + extension
}

func classifyMessage(msg *waE2E.Message) (kind string, text string) {
	if msg == nil {
		return KindOther, ""
	}
	if msg.GetConversation() != "" {
		return KindText, msg.GetConversation()
	}
	if ext := msg.GetExtendedTextMessage(); ext != nil {
		return KindText, ext.GetText()
	}
	if img := msg.GetImageMessage(); img != nil {
		return KindImage, img.GetCaption()
	}
	if vid := msg.GetVideoMessage(); vid != nil {
		return KindVideo, vid.GetCaption()
	}
	if aud := msg.GetAudioMessage(); aud != nil {
		return KindAudio, ""
	}
	if doc := msg.GetDocumentMessage(); doc != nil {
		return KindDocument, doc.GetCaption()
	}
	if wrapped := msg.GetDocumentWithCaptionMessage(); wrapped != nil {
		if doc := wrapped.GetMessage().GetDocumentMessage(); doc != nil {
			return KindDocument, doc.GetCaption()
		}
	}
	if msg.GetStickerMessage() != nil {
		return KindSticker, ""
	}
	return KindOther, ""
}

func isRelevantMessage(evt *events.Message) bool {
	if evt == nil || evt.Message == nil {
		return false
	}
	if evt.Info.Chat == types.StatusBroadcastJID {
		return false
	}
	msg := evt.Message
	if msg.GetProtocolMessage() != nil || msg.GetReactionMessage() != nil {
		return false
	}
	kind, _ := classifyMessage(msg)
	if kind == KindOther && msg.GetSenderKeyDistributionMessage() != nil {
		return false
	}
	return true
}

func messageFromEvent(evt *events.Message) Message {
	kind, text := classifyMessage(evt.Message)
	var at int64
	if !evt.Info.Timestamp.IsZero() {
		at = evt.Info.Timestamp.Unix()
	}
	return Message{
		ID:       string(evt.Info.ID),
		Chat:     evt.Info.Chat.ToNonAD().String(),
		From:     evt.Info.Sender.ToNonAD().String(),
		FromName: evt.Info.PushName,
		Text:     text,
		Mine:     evt.Info.IsFromMe,
		At:       at,
		Kind:     kind,
	}
}

func sentMessage(id string, chat types.JID, own types.JID, text string, at time.Time) Message {
	if at.IsZero() {
		at = time.Now()
	}
	return Message{
		ID:       id,
		Chat:     chat.ToNonAD().String(),
		From:     own.ToNonAD().String(),
		FromName: "",
		Text:     text,
		Mine:     true,
		At:       at.Unix(),
		Kind:     KindText,
	}
}

func savedName(info types.ContactInfo) string {
	for _, candidate := range []string{info.FullName, info.FirstName, info.BusinessName} {
		if name := strings.TrimSpace(candidate); name != "" {
			return name
		}
	}
	return ""
}

func contactName(info types.ContactInfo) string {
	if name := savedName(info); name != "" {
		return name
	}
	if name := strings.TrimSpace(info.PushName); name != "" {
		return name
	}
	return ""
}

func buildConversations(contacts map[types.JID]types.ContactInfo, groups []*types.GroupInfo) []Conversation {
	result := make([]Conversation, 0, len(contacts)+len(groups))
	for jid, info := range contacts {
		if jid.Server != types.DefaultUserServer && jid.Server != types.HiddenUserServer {
			continue
		}
		result = append(result, Conversation{
			JID:   jid.ToNonAD().String(),
			Name:  contactName(info),
			Group: false,
		})
	}
	for _, group := range groups {
		if group == nil {
			continue
		}
		name := strings.TrimSpace(group.Name)
		if name == "" {
			name = group.JID.User
		}
		result = append(result, Conversation{
			JID:   group.JID.String(),
			Name:  name,
			Group: true,
		})
	}
	sort.SliceStable(result, func(i, j int) bool {
		left := strings.ToLower(result[i].Name)
		right := strings.ToLower(result[j].Name)
		if left != right {
			return left < right
		}
		return result[i].JID < result[j].JID
	})
	return result
}
