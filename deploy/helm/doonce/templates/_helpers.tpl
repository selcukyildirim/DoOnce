{{- define "doonce.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "doonce.fullname" -}}
{{- printf "%s-%s" .Release.Name (include "doonce.name" .) | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "doonce.labels" -}}
app.kubernetes.io/name: {{ include "doonce.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
helm.sh/chart: {{ printf "%s-%s" .Chart.Name .Chart.Version }}
{{- end -}}

{{- define "doonce.selectorLabels" -}}
app.kubernetes.io/name: {{ include "doonce.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}
